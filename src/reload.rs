use std::sync::Arc;

use arc_swap::ArcSwap;
use tracing::{
    callsite,
    collect::{Collect, Interest},
    level_filters::LevelFilter,
    span::{self, Attributes, Id, Record},
    Dispatch, Event, Metadata,
};
use tracing_subscriber::subscribe::{CollectExt, Context, Layered, Subscribe};

#[derive(Default)]
pub struct ReloadableSubscriber<S, C> {
    subscriber: Arc<ArcSwap<S>>,
    collector: Arc<C>,
}

impl<S, C> ReloadableSubscriber<S, C>
where
    S: Subscribe<Arc<C>>,
    C: Collect,
{
    fn new(mut subscriber: S, collector: Arc<C>) -> Self {
        subscriber.on_subscribe(&collector);
        let subscriber = ArcSwap::from_pointee(subscriber).into();
        Self {
            subscriber,
            collector,
        }
    }

    pub fn reload(&self, mut new_subscriber: S) {
        new_subscriber.on_subscribe(&self.collector);
        self.subscriber.store(new_subscriber.into());
        callsite::rebuild_interest_cache();
        span::rebuild_filter_cache();
    }
}

impl<S, C> Clone for ReloadableSubscriber<S, C> {
    fn clone(&self) -> Self {
        Self {
            subscriber: self.subscriber.clone(),
            collector: self.collector.clone(),
        }
    }
}

macro_rules! impl_subscribe {
    ($(fn $method:ident(&self $(, $arg_name:ident: $arg_type:ty)*) $(-> $return_type:ty)?;)*) => {
        $(
            fn $method(&self $(, $arg_name: $arg_type)*) $(-> $return_type)? {
                self.subscriber.load().$method($($arg_name),*)
            }
        )*
    };
}

impl<S, C> Subscribe<Arc<C>> for ReloadableSubscriber<S, C>
where
    S: Subscribe<Arc<C>>,
    C: Collect,
{
    fn on_subscribe(&mut self, _collector: &Arc<C>) {
        // Do nothing, since `on_subscribe()` is already called in the `new()` method.
    }

    impl_subscribe!(
        fn on_register_dispatch(&self, collector: &Dispatch);
        fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, Arc<C>>);
        fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest;
        fn enabled(&self, metadata: &Metadata<'_>, ctx: Context<'_, Arc<C>>) -> bool;
        fn max_level_hint(&self) -> Option<LevelFilter>;
        fn on_record(&self, span: &Id, values: &Record<'_>, ctx: Context<'_, Arc<C>>);
        fn on_follows_from(&self, span: &Id, follows: &Id, ctx: Context<'_, Arc<C>>);
        fn event_enabled(&self, event: &Event<'_>, ctx: Context<'_, Arc<C>>) -> bool;
        fn on_event(&self, event: &Event<'_>, ctx: Context<'_, Arc<C>>);
        fn on_enter(&self, id: &Id, ctx: Context<'_, Arc<C>>);
        fn on_exit(&self, id: &Id, ctx: Context<'_, Arc<C>>);
        fn on_close(&self, id: Id, ctx: Context<'_, Arc<C>>);
        fn on_id_change(&self, old: &Id, new: &Id, ctx: Context<'_, Arc<C>>);
    );
}

type ReloadableLayered<S, C> = Layered<ReloadableSubscriber<S, C>, Arc<C>>;

pub trait WithReloadable: Collect + Sized {
    fn with_reloadable<S>(
        self,
        subscriber: S,
    ) -> (ReloadableLayered<S, Self>, ReloadableSubscriber<S, Self>)
    where
        S: Subscribe<Arc<Self>>;
}

impl<C: Collect> WithReloadable for C {
    fn with_reloadable<S>(
        self,
        subscriber: S,
    ) -> (ReloadableLayered<S, Self>, ReloadableSubscriber<S, Self>)
    where
        S: Subscribe<Arc<Self>>,
    {
        let this = Arc::new(self);
        let reloadable_subscriber = ReloadableSubscriber::new(subscriber, this.clone());
        let collector = this.with(reloadable_subscriber.clone());
        (collector, reloadable_subscriber)
    }
}
