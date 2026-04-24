use std::sync::Arc;

use crate::config::{AppConfig, Configuration};
use crate::id::IdGenerator;
use crate::logging::LogSink;
use crate::runtime::RuntimeState;

#[derive(Clone)]
pub(crate) struct HostState {
    configuration: Arc<Configuration>,
    app_config: Arc<AppConfig>,
    runtime_state: Arc<RuntimeState>,
    log_sink: Arc<dyn LogSink>,
    id_generator: Arc<dyn IdGenerator>,
}

impl HostState {
    pub(crate) fn new(
        configuration: Arc<Configuration>,
        app_config: Arc<AppConfig>,
        runtime_state: Arc<RuntimeState>,
        log_sink: Arc<dyn LogSink>,
        id_generator: Arc<dyn IdGenerator>,
    ) -> Self {
        Self {
            configuration,
            app_config,
            runtime_state,
            log_sink,
            id_generator,
        }
    }

    pub(crate) fn configuration(&self) -> &Configuration {
        self.configuration.as_ref()
    }

    pub(crate) fn app_config(&self) -> &AppConfig {
        self.app_config.as_ref()
    }

    pub(crate) fn runtime_state(&self) -> Arc<RuntimeState> {
        Arc::clone(&self.runtime_state)
    }

    pub(crate) fn log_sink(&self) -> Arc<dyn LogSink> {
        Arc::clone(&self.log_sink)
    }

    pub(crate) fn id_generator(&self) -> Arc<dyn IdGenerator> {
        Arc::clone(&self.id_generator)
    }
}
