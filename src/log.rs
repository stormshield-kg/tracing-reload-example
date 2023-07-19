use std::{
    env::{self, VarError},
    fmt,
    fs::File,
    io,
    path::Path,
    str::FromStr,
    sync::Arc,
};

use eyre::Context;
use indexmap::IndexMap;
use tracing::{dispatch, warn, Collect, Dispatch, Event};
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_subscriber::{
    filter::{EnvFilter, Filtered},
    fmt::{
        format::{Compact, DefaultFields, Format, Full, Pretty, Writer},
        FmtContext, FormatEvent, FormatFields, Subscriber,
    },
    registry::{LookupSpan, Registry},
    subscribe::{CollectExt, Layered, Subscribe},
};

use super::{
    config::{
        AppenderLogConfig, ConsoleLogConfig, ConsoleTarget, FileLogConfig, FileWritingMode,
        GlobalLogConfig, Log, LogConfig, LogConfigs, LogFormat,
    },
    reload::{ReloadableSubscriber, WithReloadable},
};

type BaseCollector<S> = Layered<S, Registry>;

type FilteredSubscriber<C> =
    Filtered<Subscriber<C, DefaultFields, EventFormat, NonBlocking>, EnvFilter, C>;

type SubscriberHandle<S> =
    ReloadableSubscriber<Vec<FilteredSubscriber<Arc<BaseCollector<S>>>>, BaseCollector<S>>;

#[must_use]
pub struct LogGuard<S> {
    subscriber_handle: SubscriberHandle<S>,
    worker_guards: Vec<WorkerGuard>,
}

#[derive(Debug)]
pub enum EventFormat {
    Full(Format<Full>),
    Pretty(Format<Pretty>),
    Compact(Format<Compact>),
    System(Format<Compact, ()>),
}

impl From<LogFormat> for EventFormat {
    fn from(format: LogFormat) -> Self {
        match format {
            LogFormat::Full => Self::Full(Format::default()),
            LogFormat::Pretty => Self::Pretty(Format::default().pretty()),
            LogFormat::Compact => Self::Compact(Format::default().compact()),
            LogFormat::System => Self::System(Format::default().compact().without_time()),
        }
    }
}

impl<C, N> FormatEvent<C, N> for EventFormat
where
    C: Collect + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, C, N>,
        writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        match self {
            EventFormat::Full(format) => format.format_event(ctx, writer, event),
            EventFormat::Pretty(format) => format.format_event(ctx, writer, event),
            EventFormat::Compact(format) => format.format_event(ctx, writer, event),
            EventFormat::System(format) => format.format_event(ctx, writer, event),
        }
    }
}

trait AppenderConfig: LogConfig {
    fn non_blocking(&self) -> io::Result<(NonBlocking, WorkerGuard)>;
}

impl AppenderConfig for ConsoleLogConfig {
    /// Create a non-blocking writer able to write logs in stdout or stderr
    fn non_blocking(&self) -> io::Result<(NonBlocking, WorkerGuard)> {
        match self.target {
            ConsoleTarget::Stdout => Ok(tracing_appender::non_blocking(std::io::stdout())),
            ConsoleTarget::Stderr => Ok(tracing_appender::non_blocking(std::io::stderr())),
        }
    }
}

impl AppenderConfig for FileLogConfig {
    /// Create a non-blocking writer able to write logs in a file
    fn non_blocking(&self) -> io::Result<(NonBlocking, WorkerGuard)> {
        let path = &self.path;

        let file = match self.mode {
            // Append to file
            FileWritingMode::Append => File::options().append(true).create(true).open(path)?,
            // Troncate and overwrite file
            FileWritingMode::Overwrite => File::create(path)?,
        };

        Ok(tracing_appender::non_blocking(file))
    }
}

struct SubscriberSetup {
    writer: NonBlocking,
    color: bool,
    filter: EnvFilter,
    format: EventFormat,
}

impl SubscriberSetup {
    fn new(writer: NonBlocking, color: bool, filter: EnvFilter, format: EventFormat) -> Self {
        Self {
            writer,
            color,
            filter,
            format,
        }
    }

    fn from_appender(
        config: &impl AppenderConfig,
        global_config: &GlobalLogConfig,
    ) -> eyre::Result<(Self, WorkerGuard)> {
        let level = global_config
            .level_from_env
            .as_deref()
            .or(config.level())
            .unwrap_or(&global_config.level);

        let color = config.color();
        let format = config.format().unwrap_or(global_config.format);
        let (non_blocking, worker_guard) = config.non_blocking()?;
        let filter = EnvFilter::from_str(level)?;
        let subscriber_setup = SubscriberSetup::new(non_blocking, color, filter, format.into());

        Ok((subscriber_setup, worker_guard))
    }

    fn into_subscriber<C>(self) -> FilteredSubscriber<C>
    where
        C: Collect + for<'a> LookupSpan<'a>,
    {
        tracing_subscriber::fmt::subscriber()
            .with_ansi(self.color)
            .with_writer(self.writer)
            .event_format(self.format)
            .with_filter(self.filter)
    }
}

#[derive(Default)]
struct Subscribers {
    subscribers: Vec<SubscriberSetup>,
    worker_guards: Vec<WorkerGuard>,
}

impl Subscribers {
    fn set_global_dispatch(collector: impl Into<Dispatch>) -> eyre::Result<()> {
        // Filter level for `tracing_log` is global and cannot be reconfigured,
        // so we inline the `init()` method to keep the default level.
        dispatch::set_global_default(collector.into())?;
        tracing_log::LogTracer::init()?;
        Ok(())
    }

    fn into_components<C>(self) -> (Vec<WorkerGuard>, Vec<FilteredSubscriber<C>>)
    where
        C: Collect + for<'a> LookupSpan<'a>,
    {
        let subscribers = self
            .subscribers
            .into_iter()
            .map(SubscriberSetup::into_subscriber)
            .collect();

        (self.worker_guards, subscribers)
    }

    fn build<S>(self, base_collector: BaseCollector<S>) -> eyre::Result<LogGuard<S>>
    where
        S: Subscribe<Registry> + Send + Sync,
    {
        let (worker_guards, subscribers) = self.into_components();
        let (collector, subscriber_handle) = base_collector.with_reloadable(subscribers);
        Self::set_global_dispatch(collector)?;

        Ok(LogGuard {
            subscriber_handle,
            worker_guards,
        })
    }
}

impl TryFrom<Log> for Subscribers {
    type Error = eyre::Error;

    fn try_from(log: Log) -> Result<Self, Self::Error> {
        let len = log.configs.appenders.len();

        let mut subscribers = Subscribers {
            subscribers: Vec::with_capacity(len),
            worker_guards: Vec::with_capacity(len),
        };

        for appender in log.configs.appenders.values() {
            let (subscriber, worker_guard) = match appender {
                AppenderLogConfig::Console(appender) => {
                    SubscriberSetup::from_appender(appender, &log.global)?
                }
                AppenderLogConfig::File(appender) => {
                    SubscriberSetup::from_appender(appender, &log.global)?
                }
            };

            subscribers.subscribers.push(subscriber);
            subscribers.worker_guards.push(worker_guard);
        }

        Ok(subscribers)
    }
}

fn build_appenders(file_contents: &str, data_dir: &Path) -> eyre::Result<Subscribers> {
    let log = Log::parse(file_contents, data_dir).context("invalid logging configuration file")?;
    Subscribers::try_from(log).context("unable to initialize appenders")
}

fn build_default_appenders() -> eyre::Result<Subscribers> {
    let level_from_env = match env::var("RUST_LOG") {
        Ok(level) => Some(level),
        Err(VarError::NotPresent) => None,
        Err(err) => return Err(err.into()),
    };

    Subscribers::try_from(Log {
        global: GlobalLogConfig {
            level_from_env,
            ..Default::default()
        },
        configs: LogConfigs {
            appenders: IndexMap::from([(
                "stdout".into(),
                AppenderLogConfig::Console(ConsoleLogConfig::default()),
            )]),
        },
    })
    .context("unable to initialize default appenders")
}

pub fn init_log<S>(
    file_contents: &str,
    data_dir: &Path,
    platform_subscriber: S,
) -> eyre::Result<LogGuard<S>>
where
    S: Subscribe<Registry> + Send + Sync,
{
    let (subscribers, error) = match build_appenders(file_contents, data_dir) {
        Ok(subscribers) => (subscribers, None),
        Err(e) => (build_default_appenders()?, Some(e)),
    };

    let base_collector = tracing_subscriber::registry().with(platform_subscriber);
    let log_guard = subscribers.build(base_collector)?;

    if let Some(error) = error {
        warn!(%error, "Using default logging configuration");
    }

    Ok(log_guard)
}

pub fn reload_log<S>(
    file_contents: &str,
    data_dir: &Path,
    mut log_guard: LogGuard<S>,
) -> eyre::Result<LogGuard<S>>
where
    S: Subscribe<Registry> + Send + Sync,
{
    // Flush and clear current appenders
    log_guard.worker_guards.clear();

    let (subscribers, error) = match build_appenders(file_contents, data_dir) {
        Ok(subscribers) => (subscribers, None),
        Err(e) => (build_default_appenders()?, Some(e)),
    };

    let (worker_guards, subscribers) = subscribers.into_components();
    log_guard.subscriber_handle.reload(subscribers);

    if let Some(error) = error {
        warn!(%error, "Using default logging configuration");
    }

    Ok(LogGuard {
        worker_guards,
        ..log_guard
    })
}
