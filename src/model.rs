use std::cmp::Ordering;

pub(crate) const CORE_GROUP_DIRECTORY_NAME: &str = "core";

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ApiGroup {
    Core,
    Named(String),
}

impl ApiGroup {
    pub(crate) fn name(&self) -> &str {
        match self {
            ApiGroup::Core => CORE_GROUP_DIRECTORY_NAME,
            ApiGroup::Named(group) => group,
        }
    }
}

impl Ord for ApiGroup {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (ApiGroup::Core, ApiGroup::Named(group)) if group == CORE_GROUP_DIRECTORY_NAME => {
                Ordering::Greater
            }
            (ApiGroup::Named(group), ApiGroup::Core) if group == CORE_GROUP_DIRECTORY_NAME => {
                Ordering::Less
            }
            _ => self.name().cmp(other.name()),
        }
    }
}

impl PartialOrd for ApiGroup {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ApiVersion {
    group: ApiGroup,
    version: String,
}

impl ApiVersion {
    pub(crate) fn new(group: &ApiGroup, version: &str) -> Self {
        Self {
            group: group.clone(),
            version: version.to_string(),
        }
    }

    pub(crate) fn group(&self) -> &ApiGroup {
        &self.group
    }

    pub(crate) fn version(&self) -> &str {
        &self.version
    }

    pub(crate) fn name(&self) -> &str {
        self.version()
    }

    pub(crate) fn api_version_string(&self) -> String {
        match self.group() {
            ApiGroup::Core => self.version().to_string(),
            ApiGroup::Named(group) => format!("{}/{}", group, self.version()),
        }
    }
}

impl Ord for ApiVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        let ord = self.group().cmp(other.group());
        match ord {
            Ordering::Equal => self.name().cmp(other.name()),
            _ => ord,
        }
    }
}

impl PartialOrd for ApiVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub(crate) enum ResourceScope {
    Cluster,
    Namespaced,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub(crate) struct ApiResource {
    version: ApiVersion,
    name: String,
    kind: String,
    scope: ResourceScope,
}

impl ApiResource {
    pub(crate) fn new(version: &ApiVersion, name: &str, kind: &str, scope: ResourceScope) -> Self {
        Self {
            version: version.clone(),
            name: name.to_string(),
            kind: kind.to_string(),
            scope,
        }
    }

    pub(crate) fn version(&self) -> &ApiVersion {
        &self.version
    }

    pub(crate) fn resource(&self) -> &str {
        &self.name
    }

    pub(crate) fn kind(&self) -> &str {
        &self.kind
    }

    pub(crate) fn scope(&self) -> ResourceScope {
        self.scope
    }

    pub(crate) fn name(&self) -> &str {
        self.resource()
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct NamespaceId {
    resource: ApiResource,
    name: String,
}

impl NamespaceId {
    pub(crate) fn new(resource: &ApiResource, name: &str) -> Self {
        Self {
            resource: resource.clone(),
            name: name.to_string(),
        }
    }

    pub(crate) fn resource(&self) -> &ApiResource {
        &self.resource
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }
}

impl Ord for NamespaceId {
    fn cmp(&self, other: &Self) -> Ordering {
        let ord = self.resource().cmp(other.resource());
        match ord {
            Ordering::Equal => self.name().cmp(other.name()),
            _ => ord,
        }
    }
}

impl PartialOrd for NamespaceId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ObjectId {
    resource: ApiResource,
    name: String,
    namespace: Option<String>,
}

impl ObjectId {
    pub(crate) fn new(resource: &ApiResource, name: &str, namespace: Option<&str>) -> Self {
        Self {
            resource: resource.clone(),
            name: name.to_string(),
            namespace: namespace.map(|s| s.to_string()),
        }
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    pub(crate) fn resource(&self) -> &ApiResource {
        &self.resource
    }
}

impl Ord for ObjectId {
    fn cmp(&self, other: &Self) -> Ordering {
        let ord = self.resource().cmp(other.resource());
        match (ord, self.namespace(), other.namespace()) {
            (Ordering::Equal, None, None) => self.name().cmp(other.name()),
            (Ordering::Equal, None, Some(_)) => Ordering::Greater,
            (Ordering::Equal, Some(_), None) => Ordering::Less,
            (Ordering::Equal, Some(ns_self), Some(ns_other)) => {
                let ord = ns_self.cmp(ns_other);
                match ord {
                    Ordering::Equal => self.name().cmp(other.name()),
                    _ => ord,
                }
            }
            _ => ord,
        }
    }
}

impl PartialOrd for ObjectId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Node {
    Root,
    Group(ApiGroup),
    Version(ApiVersion),
    Resource(ApiResource),
    Namespace(NamespaceId),
    Object(ObjectId),
}

impl Node {
    pub(crate) fn name(&self) -> &str {
        match self {
            Node::Root => "",
            Node::Group(group) => group.name(),
            Node::Version(version) => version.name(),
            Node::Resource(resource) => resource.name(),
            Node::Namespace(namespace) => namespace.name(),
            Node::Object(object) => object.name(),
        }
    }

    pub(crate) fn parent(&self) -> Option<Node> {
        match self {
            Node::Root => None,
            Node::Group(_) => Some(Node::Root),
            Node::Version(version) => Some(Node::Group(version.group().clone())),
            Node::Resource(resource) => Some(Node::Version(resource.version().clone())),
            Node::Namespace(namespace) => Some(Node::Resource(namespace.resource().clone())),
            Node::Object(object) => Some(match object.namespace() {
                Some(namespace) => Node::Namespace(NamespaceId::new(object.resource(), namespace)),
                None => Node::Resource(object.resource().clone()),
            }),
        }
    }
}
