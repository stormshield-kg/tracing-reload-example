mod config;
mod log;
mod reload;

use std::fs;
use std::path::Path;
use std::time::Duration;

use tracing::{debug, debug_span, error, info, trace, trace_span, warn};
use tracing_subscriber::subscribe::Identity;

use self::log::{init_log, reload_log};

fn main() -> eyre::Result<()> {
    let data_dir = Path::new("data");
    fs::create_dir_all(data_dir)?;

    let mut log_guard = init_log(r#"[log]"#, data_dir, Identity::new())?;

    let _span = trace_span!("trace_span0").entered();

    info!("info 0");

    let file_contents = r#"
        [log.appenders.stdout]
        kind = "console"
        level = "trace"
        color = false
    "#;
    log_guard = reload_log(file_contents, data_dir, log_guard)?;

    {
        let _span = trace_span!("trace_span1").entered();

        for _ in 0..2 {
            let _span = debug_span!("debug_span2").entered();

            trace!("trace 1");
            debug!("debug 1");
            info!("info 1");
            warn!("warn 1");
            error!("error 1");

            let file_contents = r#"
                [log.appenders.log1]
                kind = "file"
                level = "warn"
                path = "log1.log"

                [log.appenders.log2]
                kind = "file"
                level = "debug"
                path = "log2.log"
            "#;
            log_guard = reload_log(file_contents, data_dir, log_guard)?;

            trace!("trace 2");
            debug!("debug 2");
            info!("info 2");
            warn!("warn 2");
            error!("error 2");
        }

        let file_contents = r#"
            [log.appenders.log1]
            kind = "file"
            level = "trace"
            path = "log1.log"

            [log.appenders.log2]
            kind = "file"
            level = "warn"
            path = "log2.log"
        "#;
        log_guard = reload_log(file_contents, data_dir, log_guard)?;

        {
            let _span = debug_span!("debug_span3").entered();

            trace!("trace 3");
            debug!("debug 3");
            info!("info 3");
            warn!("warn 3");
            error!("error 3");

            let file_contents = r#"
                [log.appenders.log1]
                kind = "file"
                level = "error"
                path = "log1.log"

                [log.appenders.log2]
                kind = "file"
                level = "error"
                path = "log2.log"
            "#;
            log_guard = reload_log(file_contents, data_dir, log_guard)?;

            trace!("trace 4");
            debug!("debug 4");
            info!("info 4");
            warn!("warn 4");
            error!("error 4");
        }
    }

    std::thread::spawn(|| {
        trace!("trace 5");
        debug!("debug 5");
        info!("info 5");
        warn!("warn 5");
        error!("error 5");
    })
    .join()
    .unwrap();

    std::thread::sleep(Duration::from_millis(100));

    let file_contents = r#"
        [log.appenders.log2]
        kind = "file"
        level = "debug"
        path = "log2.log"
    "#;
    log_guard = reload_log(file_contents, data_dir, log_guard)?;

    trace!("trace 6");
    debug!("debug 6");
    info!("info 6");
    warn!("warn 6");
    error!("error 6");

    drop(log_guard);
    Ok(())
}
