use std::sync::{Mutex, PoisonError};

use bimap::BiMap;
use fuser::INodeNo;
use thiserror::Error;

use crate::model::Node;

const FIRST_DYNAMIC_INODE: u64 = 2;

#[derive(Debug, Error)]
pub(super) enum InodeRegistryError {
    #[error("inode allocation exhausted")]
    InodeExhausted,

    #[error("inode registry lock poisoned")]
    LockPoisoned,
}

impl<T> From<PoisonError<T>> for InodeRegistryError {
    fn from(_: PoisonError<T>) -> Self {
        InodeRegistryError::LockPoisoned
    }
}

struct State {
    entries: BiMap<Node, INodeNo>,
    next_inode: u64,
}

impl State {
    fn allocate_inode(&mut self) -> Result<INodeNo, InodeRegistryError> {
        let inode = INodeNo(self.next_inode);
        self.next_inode = self
            .next_inode
            .checked_add(1)
            .ok_or(InodeRegistryError::InodeExhausted)?;
        Ok(inode)
    }
}

pub(super) struct InodeRegistry {
    state: Mutex<State>,
}

impl InodeRegistry {
    pub(super) fn new() -> Self {
        let mut entries = BiMap::new();
        entries.insert(Node::Root, INodeNo::ROOT);

        Self {
            state: Mutex::new(State {
                entries,
                next_inode: FIRST_DYNAMIC_INODE,
            }),
        }
    }

    pub(super) fn get_or_create_inode(&self, node: Node) -> Result<INodeNo, InodeRegistryError> {
        let mut state = self.state.lock()?;
        if let Some(inode) = state.entries.get_by_left(&node) {
            return Ok(*inode);
        }

        let inode = state.allocate_inode()?;
        state.entries.insert(node, inode);
        Ok(inode)
    }

    pub(super) fn node_for_inode(
        &self,
        inode: INodeNo,
    ) -> Result<Option<Node>, InodeRegistryError> {
        let state = self.state.lock()?;
        Ok(state.entries.get_by_right(&inode).cloned())
    }

    pub(super) fn inode_for_node(
        &self,
        node: &Node,
    ) -> Result<Option<INodeNo>, InodeRegistryError> {
        let state = self.state.lock()?;
        Ok(state.entries.get_by_left(node).copied())
    }
}
