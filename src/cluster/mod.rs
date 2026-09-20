use ::kube::Error as KubeError;
use thiserror::Error;

use crate::model::{Node, ObjectId};

mod kube;

pub(crate) use kube::KubeClusterReader;

#[derive(Debug, Error)]
pub(crate) enum ClusterError {
    #[error("Kubernetes API request failed: {0}")]
    Api(KubeError),

    #[error("node cannot have children")]
    CannotHaveChildren,

    #[error("expected a namespaced API resource")]
    ExpectedNamespacedResource,

    #[error("cluster resource not found")]
    NotFound,

    #[error("unexpected cluster response: {0}")]
    Unexpected(String),
}

#[async_trait::async_trait]
pub(crate) trait ClusterReader: Send + Sync {
    async fn children(&self, parent: &Node) -> Result<Vec<Node>, ClusterError>;
    async fn child(&self, parent: &Node, name: &str) -> Result<Node, ClusterError>;
    async fn object_data(&self, object: &ObjectId) -> Result<Vec<u8>, ClusterError>;
}
