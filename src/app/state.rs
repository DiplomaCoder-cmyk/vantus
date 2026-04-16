use std::any::{Any, TypeId, type_name};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, RwLock};

type DynService = Arc<dyn Any + Send + Sync>;
type ServiceFactory = Arc<
    dyn Fn(&ServiceScope, &mut ResolutionStack) -> Result<DynService, ServiceError> + Send + Sync,
>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceLifetime {
    Singleton,
    Scoped,
    Transient,
}

#[derive(Debug, Clone)]
pub enum ServiceError {
    Missing { type_name: &'static str },
    CircularDependency { chain: Vec<&'static str> },
    ScopedFromRoot { type_name: &'static str },
    TypeMismatch { type_name: &'static str },
    Factory(String),
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServiceError::Missing { type_name } => write!(f, "service not registered: {type_name}"),
            ServiceError::CircularDependency { chain } => {
                write!(f, "circular service dependency: {}", chain.join(" -> "))
            }
            ServiceError::ScopedFromRoot { type_name } => {
                write!(f, "scoped service resolved from root provider: {type_name}")
            }
            ServiceError::TypeMismatch { type_name } => {
                write!(f, "service type mismatch for {type_name}")
            }
            ServiceError::Factory(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ServiceError {}

struct ServiceDescriptor {
    lifetime: ServiceLifetime,
    type_name: &'static str,
    factory: ServiceFactory,
}

#[derive(Default)]
struct ResolutionStack {
    chain: Vec<TypeId>,
    names: Vec<&'static str>,
}

impl ResolutionStack {
    fn enter(&mut self, type_id: TypeId, type_name: &'static str) -> Result<(), ServiceError> {
        if self.chain.contains(&type_id) {
            let mut names = self.names.clone();
            names.push(type_name);
            return Err(ServiceError::CircularDependency { chain: names });
        }
        self.chain.push(type_id);
        self.names.push(type_name);
        Ok(())
    }

    fn exit(&mut self) {
        self.chain.pop();
        self.names.pop();
    }
}

pub struct ServiceCollection {
    descriptors: HashMap<TypeId, ServiceDescriptor>,
}

impl ServiceCollection {
    pub fn new() -> Self {
        Self {
            descriptors: HashMap::new(),
        }
    }

    pub fn contains<T: Send + Sync + 'static>(&self) -> bool {
        self.descriptors.contains_key(&TypeId::of::<T>())
    }

    pub fn add_singleton<T>(&mut self, value: T) -> &mut Self
    where
        T: Send + Sync + 'static,
    {
        let value: DynService = Arc::new(value);
        self.add_descriptor::<T>(ServiceLifetime::Singleton, move |_scope, _stack| {
            Ok(Arc::clone(&value))
        });
        self
    }

    pub fn add_singleton_with<T, F>(&mut self, factory: F) -> &mut Self
    where
        T: Send + Sync + 'static,
        F: Fn(&ServiceScope) -> Result<T, ServiceError> + Send + Sync + 'static,
    {
        self.add_descriptor::<T>(ServiceLifetime::Singleton, move |scope, _stack| {
            factory(scope).map(|value| Arc::new(value) as DynService)
        });
        self
    }

    pub fn add_scoped<T, F>(&mut self, factory: F) -> &mut Self
    where
        T: Send + Sync + 'static,
        F: Fn(&ServiceScope) -> Result<T, ServiceError> + Send + Sync + 'static,
    {
        self.add_descriptor::<T>(ServiceLifetime::Scoped, move |scope, _stack| {
            factory(scope).map(|value| Arc::new(value) as DynService)
        });
        self
    }

    pub fn add_transient<T, F>(&mut self, factory: F) -> &mut Self
    where
        T: Send + Sync + 'static,
        F: Fn(&ServiceScope) -> Result<T, ServiceError> + Send + Sync + 'static,
    {
        self.add_descriptor::<T>(ServiceLifetime::Transient, move |scope, _stack| {
            factory(scope).map(|value| Arc::new(value) as DynService)
        });
        self
    }

    pub fn build(self) -> ServiceContainer {
        ServiceContainer {
            descriptors: self.descriptors,
            singletons: RwLock::new(HashMap::new()),
        }
    }

    fn add_descriptor<T>(
        &mut self,
        lifetime: ServiceLifetime,
        factory: impl Fn(&ServiceScope, &mut ResolutionStack) -> Result<DynService, ServiceError>
        + Send
        + Sync
        + 'static,
    ) where
        T: Send + Sync + 'static,
    {
        self.descriptors.insert(
            TypeId::of::<T>(),
            ServiceDescriptor {
                lifetime,
                type_name: type_name::<T>(),
                factory: Arc::new(factory),
            },
        );
    }
}

impl Default for ServiceCollection {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ServiceContainer {
    descriptors: HashMap<TypeId, ServiceDescriptor>,
    singletons: RwLock<HashMap<TypeId, DynService>>,
}

impl ServiceContainer {
    pub fn create_scope(self: &Arc<Self>) -> ServiceScope {
        ServiceScope {
            container: Arc::clone(self),
            scoped: Mutex::new(HashMap::new()),
            allow_scoped: true,
        }
    }

    pub fn root_scope(self: &Arc<Self>) -> ServiceScope {
        ServiceScope {
            container: Arc::clone(self),
            scoped: Mutex::new(HashMap::new()),
            allow_scoped: false,
        }
    }
}

pub struct ServiceScope {
    container: Arc<ServiceContainer>,
    scoped: Mutex<HashMap<TypeId, DynService>>,
    allow_scoped: bool,
}

impl ServiceScope {
    pub fn resolve<T>(&self) -> Result<Arc<T>, ServiceError>
    where
        T: Send + Sync + 'static,
    {
        let mut stack = ResolutionStack::default();
        self.resolve_with_stack::<T>(&mut stack)
    }

    fn resolve_with_stack<T>(&self, stack: &mut ResolutionStack) -> Result<Arc<T>, ServiceError>
    where
        T: Send + Sync + 'static,
    {
        let type_id = TypeId::of::<T>();
        let type_name = type_name::<T>();
        let value = self.resolve_by_id(type_id, type_name, stack)?;
        value
            .downcast::<T>()
            .map_err(|_| ServiceError::TypeMismatch { type_name })
    }

    fn resolve_by_id(
        &self,
        type_id: TypeId,
        type_name: &'static str,
        stack: &mut ResolutionStack,
    ) -> Result<DynService, ServiceError> {
        let descriptor = self
            .container
            .descriptors
            .get(&type_id)
            .ok_or(ServiceError::Missing { type_name })?;

        stack.enter(type_id, descriptor.type_name)?;
        let result = match descriptor.lifetime {
            ServiceLifetime::Singleton => {
                if let Some(existing) = self
                    .container
                    .singletons
                    .read()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .get(&type_id)
                    .cloned()
                {
                    Ok(existing)
                } else {
                    let created = (descriptor.factory)(self, stack)?;
                    self.container
                        .singletons
                        .write()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .insert(type_id, Arc::clone(&created));
                    Ok(created)
                }
            }
            ServiceLifetime::Scoped => {
                if !self.allow_scoped {
                    Err(ServiceError::ScopedFromRoot {
                        type_name: descriptor.type_name,
                    })
                } else if let Some(existing) = self
                    .scoped
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .get(&type_id)
                    .cloned()
                {
                    Ok(existing)
                } else {
                    let created = (descriptor.factory)(self, stack)?;
                    self.scoped
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .insert(type_id, Arc::clone(&created));
                    Ok(created)
                }
            }
            ServiceLifetime::Transient => (descriptor.factory)(self, stack),
        };
        stack.exit();
        result
    }
}

#[derive(Clone)]
pub struct Service<T>(Arc<T>);

impl<T> Service<T> {
    pub fn into_inner(self) -> Arc<T> {
        self.0
    }
}

impl<T> From<Arc<T>> for Service<T> {
    fn from(value: Arc<T>) -> Self {
        Self(value)
    }
}

impl<T> AsRef<T> for Service<T> {
    fn as_ref(&self) -> &T {
        &self.0
    }
}

#[derive(Clone)]
pub struct Config<T>(Arc<T>);

impl<T> Config<T> {
    pub fn into_inner(self) -> Arc<T> {
        self.0
    }
}

impl<T> From<Arc<T>> for Config<T> {
    fn from(value: Arc<T>) -> Self {
        Self(value)
    }
}

impl<T> AsRef<T> for Config<T> {
    fn as_ref(&self) -> &T {
        &self.0
    }
}
