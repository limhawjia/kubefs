use std::iter;

use itertools::Itertools;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::APIResource as DiscoveryApiResource;
use kube::{
    Api, Client, Error as KubeError, Resource,
    api::{
        ApiResource as DynamicApiResource, DynamicObject, GetParams, ListParams,
        Request as KubeRequest,
    },
};

use crate::{
    cluster::{ClusterError, ClusterReader},
    model::{
        ApiGroup, ApiResource, ApiVersion, CORE_GROUP_DIRECTORY_NAME, NamespaceId, Node, ObjectId,
        ResourceScope,
    },
};

impl From<KubeError> for ClusterError {
    fn from(err: KubeError) -> Self {
        match err {
            KubeError::Api(status) if status.is_not_found() => ClusterError::NotFound,
            other => ClusterError::Api(other),
        }
    }
}

pub(crate) struct KubeClusterReader {
    client: Client,
}

#[async_trait::async_trait]
impl ClusterReader for KubeClusterReader {
    async fn children(&self, parent: &Node) -> Result<Vec<Node>, ClusterError> {
        tracing::trace!(
            operation = "cluster.children",
            ?parent,
            "requesting cluster children",
        );

        match parent {
            Node::Root => self.list_groups().await,
            Node::Group(group) => self.list_versions(group).await,
            Node::Version(version) => self.list_resources(version).await,
            Node::Resource(resource) => match resource.scope() {
                ResourceScope::Cluster => self.list_cluster_objects(resource).await,
                ResourceScope::Namespaced => self.list_namespaces(resource).await,
            },
            Node::Namespace(namespace) => self.list_namespaced_objects(namespace).await,
            Node::Object(_) => Err(ClusterError::CannotHaveChildren),
        }
    }

    async fn child(&self, parent: &Node, name: &str) -> Result<Node, ClusterError> {
        tracing::trace!(
            operation = "cluster.child",
            ?parent,
            child_name = name,
            "requesting cluster child",
        );

        match parent {
            Node::Root => self.get_group(name).await,
            Node::Group(group) => self.get_version(group, name).await,
            Node::Version(version) => self.get_resource(version, name).await,
            Node::Resource(resource) => match resource.scope() {
                ResourceScope::Cluster => self.get_cluster_object(resource, name).await,
                ResourceScope::Namespaced => self.get_namespace(resource, name).await,
            },
            Node::Namespace(namespace_id) => self.get_namespaced_object(namespace_id, name).await,
            Node::Object(_) => Err(ClusterError::CannotHaveChildren),
        }
    }

    async fn object_data(&self, object: &ObjectId) -> Result<Vec<u8>, ClusterError> {
        tracing::trace!(
            operation = "cluster.object_data",
            ?object,
            "requesting Kubernetes object data",
        );

        let url_path = DynamicObject::url_path(
            &object.resource().dynamic_api_resource(),
            object.namespace(),
        );

        let mut req = KubeRequest::new(url_path)
            .get(object.name(), &GetParams::default())
            .map_err(|err| ClusterError::Unexpected(format!("failed to create request: {err}")))?;

        req.headers_mut().insert(
            "Accept",
            "application/yaml".parse().map_err(|err| {
                ClusterError::Unexpected(format!("failed to create request: {err}"))
            })?,
        );

        Ok(self.client.request_text(req).await?.into_bytes())
    }
}

impl KubeClusterReader {
    pub(crate) fn new(client: Client) -> Self {
        Self { client }
    }

    async fn list_groups(&self) -> Result<Vec<Node>, ClusterError> {
        let results = self.client.list_api_groups().await?.groups;
        if results
            .iter()
            .any(|group| group.name == CORE_GROUP_DIRECTORY_NAME)
        {
            return Err(ClusterError::Unexpected(
                "discovered API group conflicts with reserved name core".to_string(),
            ));
        }

        let groups = iter::once(ApiGroup::Core).chain(
            results
                .into_iter()
                .map(|group| group.name)
                .sorted()
                .map(ApiGroup::Named),
        );

        Ok(groups.sorted().map(Node::Group).collect())
    }

    async fn get_group(&self, name: &str) -> Result<Node, ClusterError> {
        let results = self.client.list_api_groups().await?.groups;
        if results
            .iter()
            .any(|group| group.name == CORE_GROUP_DIRECTORY_NAME)
        {
            return Err(ClusterError::Unexpected(
                "discovered API group conflicts with reserved name core".to_string(),
            ));
        }

        if name == CORE_GROUP_DIRECTORY_NAME {
            Ok(Node::Group(ApiGroup::Core))
        } else {
            results
                .into_iter()
                .find(|group| group.name == name)
                .map(|group| ApiGroup::Named(group.name))
                .map(Node::Group)
                .ok_or(ClusterError::NotFound)
        }
    }

    async fn list_versions(&self, group: &ApiGroup) -> Result<Vec<Node>, ClusterError> {
        let versions = match group {
            ApiGroup::Core => self.client.list_core_api_versions().await?.versions,
            ApiGroup::Named(name) => {
                let Some(group) = self
                    .client
                    .list_api_groups()
                    .await?
                    .groups
                    .into_iter()
                    .find(|group| &group.name == name)
                else {
                    return Err(ClusterError::NotFound);
                };
                group
                    .versions
                    .into_iter()
                    .map(|version| version.version)
                    .collect()
            }
        };
        Ok(versions
            .into_iter()
            .map(|version| ApiVersion::new(group, &version))
            .sorted()
            .map(Node::Version)
            .collect())
    }

    async fn get_version(&self, group: &ApiGroup, name: &str) -> Result<Node, ClusterError> {
        match group {
            ApiGroup::Core => self.client.list_core_api_resources(name).await?,
            ApiGroup::Named(group) => {
                self.client
                    .list_api_group_resources(&format!("{}/{}", group, name))
                    .await?
            }
        };
        Ok(Node::Version(ApiVersion::new(group, name)))
    }

    async fn list_resources(&self, version: &ApiVersion) -> Result<Vec<Node>, ClusterError> {
        let resources = match version.group() {
            ApiGroup::Core => {
                self.client
                    .list_core_api_resources(&version.api_version_string())
                    .await?
            }
            ApiGroup::Named(_) => {
                self.client
                    .list_api_group_resources(&version.api_version_string())
                    .await?
            }
        };
        Ok(resources
            .resources
            .into_iter()
            .filter(is_projectable_resource)
            .map(|resource| {
                let scope = if resource.namespaced {
                    ResourceScope::Namespaced
                } else {
                    ResourceScope::Cluster
                };
                ApiResource::new(version, &resource.name, &resource.kind, scope)
            })
            .sorted()
            .map(Node::Resource)
            .collect())
    }

    async fn get_resource(&self, version: &ApiVersion, name: &str) -> Result<Node, ClusterError> {
        let resources = match version.group() {
            ApiGroup::Core => {
                self.client
                    .list_core_api_resources(version.version())
                    .await?
            }
            ApiGroup::Named(_) => {
                self.client
                    .list_api_group_resources(&version.api_version_string())
                    .await?
            }
        };
        resources
            .resources
            .into_iter()
            .filter(is_projectable_resource)
            .find(|resource| resource.name == name)
            .map(|resource| {
                ApiResource::new(
                    version,
                    &resource.name,
                    &resource.kind,
                    if resource.namespaced {
                        ResourceScope::Namespaced
                    } else {
                        ResourceScope::Cluster
                    },
                )
            })
            .map(Node::Resource)
            .ok_or(ClusterError::NotFound)
    }

    async fn list_namespaces(&self, resource: &ApiResource) -> Result<Vec<Node>, ClusterError> {
        if let ResourceScope::Cluster = resource.scope() {
            return Err(ClusterError::ExpectedNamespacedResource);
        }

        let dynamic_api_resource = resource.dynamic_api_resource();
        let api: Api<DynamicObject> = Api::all_with(self.client.clone(), &dynamic_api_resource);

        let metadata = api.list_metadata(&ListParams::default()).await?;
        let namespaces = metadata
            .into_iter()
            .map(|metadata| {
                metadata.metadata.namespace.ok_or(ClusterError::Unexpected(
                    "missing namespace metadata".to_string(),
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(namespaces
            .into_iter()
            .unique()
            .map(|namespace| NamespaceId::new(resource, &namespace))
            .sorted()
            .map(Node::Namespace)
            .collect())
    }

    async fn get_namespace(
        &self,
        resource: &ApiResource,
        name: &str,
    ) -> Result<Node, ClusterError> {
        if let ResourceScope::Cluster = resource.scope() {
            return Err(ClusterError::ExpectedNamespacedResource);
        }

        let dynamic_api_resource = resource.dynamic_api_resource();
        let api: Api<DynamicObject> =
            Api::namespaced_with(self.client.clone(), name, &dynamic_api_resource);

        let params = ListParams::default().limit(1);
        let result = api.list_metadata(&params).await?;

        if result.items.is_empty() {
            Err(ClusterError::NotFound)
        } else {
            Ok(Node::Namespace(NamespaceId::new(resource, name)))
        }
    }

    async fn list_cluster_objects(
        &self,
        resource: &ApiResource,
    ) -> Result<Vec<Node>, ClusterError> {
        let dynamic_api_resource = resource.dynamic_api_resource();
        let api: Api<DynamicObject> = Api::all_with(self.client.clone(), &dynamic_api_resource);

        let metadata = api.list_metadata(&ListParams::default()).await?;
        let names = metadata
            .into_iter()
            .map(|m| {
                m.metadata.name.ok_or(ClusterError::Unexpected(
                    "missing name metadata".to_string(),
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(names
            .into_iter()
            .map(|name| ObjectId::new(resource, &name, None))
            .sorted()
            .map(Node::Object)
            .collect())
    }

    async fn get_cluster_object(
        &self,
        resource: &ApiResource,
        name: &str,
    ) -> Result<Node, ClusterError> {
        let dynamic_api_resource = resource.dynamic_api_resource();
        let api: Api<DynamicObject> = Api::all_with(self.client.clone(), &dynamic_api_resource);
        api.get_metadata(name).await?;
        Ok(Node::Object(ObjectId::new(resource, name, None)))
    }

    async fn list_namespaced_objects(
        &self,
        namespace: &NamespaceId,
    ) -> Result<Vec<Node>, ClusterError> {
        let dynamic_api_resource = namespace.resource().dynamic_api_resource();
        let api: Api<DynamicObject> =
            Api::namespaced_with(self.client.clone(), namespace.name(), &dynamic_api_resource);

        let metadata = api.list_metadata(&ListParams::default()).await?;
        let names = metadata
            .into_iter()
            .map(|m| {
                m.metadata.name.ok_or(ClusterError::Unexpected(
                    "missing name metadata".to_string(),
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(names
            .into_iter()
            .map(|name| ObjectId::new(namespace.resource(), &name, Some(namespace.name())))
            .sorted()
            .map(Node::Object)
            .collect())
    }

    async fn get_namespaced_object(
        &self,
        namespace: &NamespaceId,
        name: &str,
    ) -> Result<Node, ClusterError> {
        let dynamic_api_resource = namespace.resource().dynamic_api_resource();
        let api: Api<DynamicObject> =
            Api::namespaced_with(self.client.clone(), namespace.name(), &dynamic_api_resource);
        api.get_metadata(name).await?;
        Ok(Node::Object(ObjectId::new(
            namespace.resource(),
            name,
            Some(namespace.name()),
        )))
    }
}

impl ApiResource {
    fn dynamic_api_resource(&self) -> DynamicApiResource {
        let group = match self.version().group() {
            ApiGroup::Core => "",
            ApiGroup::Named(group) => group,
        };

        DynamicApiResource {
            group: group.to_string(),
            version: self.version().name().to_string(),
            api_version: self.version().api_version_string(),
            kind: self.kind().to_string(),
            plural: self.name().to_string(),
        }
    }
}

fn is_projectable_resource(resource: &DiscoveryApiResource) -> bool {
    !resource.name.contains('/')
        && resource.verbs.iter().any(|verb| verb == "list")
        && resource.verbs.iter().any(|verb| verb == "get")
}
