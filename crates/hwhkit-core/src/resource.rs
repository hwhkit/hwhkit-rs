use async_trait::async_trait;
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::{Arc, RwLock},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    Sql,
    Kv,
    Vector,
    Search,
    MessageBus,
    Custom(String),
}

impl ResourceKind {
    pub fn custom(name: impl Into<String>) -> Self {
        Self::Custom(name.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ResourceKey {
    kind: ResourceKind,
    name: String,
    type_id: TypeId,
}

#[derive(Clone, Default)]
pub struct ResourceRegistry {
    values: Arc<RwLock<HashMap<ResourceKey, Arc<dyn Any + Send + Sync>>>>,
}

impl ResourceRegistry {
    pub fn register<T>(&self, kind: ResourceKind, name: impl Into<String>, value: T)
    where
        T: Send + Sync + 'static,
    {
        let key = ResourceKey {
            kind,
            name: name.into(),
            type_id: TypeId::of::<T>(),
        };

        self.values
            .write()
            .expect("resource registry lock poisoned")
            .insert(key, Arc::new(value));
    }

    pub fn get<T>(&self, kind: ResourceKind, name: &str) -> Option<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        let key = ResourceKey {
            kind,
            name: name.to_string(),
            type_id: TypeId::of::<T>(),
        };

        let values = self.values.read().expect("resource registry lock poisoned");
        let value = values.get(&key)?;
        Arc::clone(value).downcast::<T>().ok()
    }

    pub fn contains<T>(&self, kind: ResourceKind, name: &str) -> bool
    where
        T: Send + Sync + 'static,
    {
        self.get::<T>(kind, name).is_some()
    }
}

#[async_trait]
pub trait SqlDatabase: Send + Sync {
    async fn execute(&self, statement: &str, params: &[String]) -> crate::Result<u64>;
    async fn query(&self, statement: &str, params: &[String])
        -> crate::Result<Vec<ResourceRecord>>;
    async fn health_check(&self) -> crate::Result<()>;
}

#[async_trait]
pub trait KvStore: Send + Sync {
    async fn get(&self, key: &str) -> crate::Result<Option<Vec<u8>>>;
    async fn set(&self, key: &str, value: Vec<u8>) -> crate::Result<()>;
    async fn delete(&self, key: &str) -> crate::Result<()>;
    async fn health_check(&self) -> crate::Result<()>;
}

#[async_trait]
pub trait VectorStore: Send + Sync {
    async fn upsert(&self, collection: &str, points: Vec<VectorRecord>) -> crate::Result<()>;
    async fn search(
        &self,
        collection: &str,
        vector: Vec<f32>,
        limit: usize,
    ) -> crate::Result<Vec<VectorSearchHit>>;
    async fn health_check(&self) -> crate::Result<()>;
}

#[async_trait]
pub trait SearchEngine: Send + Sync {
    async fn index(&self, index: &str, documents: Vec<ResourceRecord>) -> crate::Result<()>;
    async fn search(&self, index: &str, query: &str) -> crate::Result<Vec<ResourceRecord>>;
    async fn health_check(&self) -> crate::Result<()>;
}

#[async_trait]
pub trait MessageBusResource: Send + Sync {
    async fn publish(&self, topic: &str, payload: Vec<u8>) -> crate::Result<()>;
    async fn request(&self, topic: &str, payload: Vec<u8>) -> crate::Result<Vec<u8>>;
    async fn health_check(&self) -> crate::Result<()>;
}

#[derive(Debug, Clone, Default)]
pub struct ResourceRecord {
    pub fields: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct VectorRecord {
    pub id: String,
    pub vector: Vec<f32>,
    pub payload: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct VectorSearchHit {
    pub id: String,
    pub score: f32,
    pub payload: HashMap<String, String>,
}
