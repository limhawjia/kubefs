use kube::Client;
use tokio::runtime::Handle;

mod cluster;
mod fuse;
mod model;

pub use fuse::KubeFs;

pub fn new_from_kube_client(client: Client, handle: Handle) -> KubeFs {
    let reader = cluster::KubeClusterReader::new(client);
    KubeFs::new(Box::new(reader), handle)
}
