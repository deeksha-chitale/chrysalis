use std::collections::HashMap;
use crate::storable::value::{PerlValue, ValueRef}; 

pub struct SerealSeenTable {
    offsets: HashMap<usize, ValueRef>,
}

impl SerealSeenTable {
    pub fn new() -> Self {
        Self { offsets: HashMap::new() }
    }

    pub fn register(&mut self, offset: usize, value: ValueRef) {
        self.offsets.insert(offset, value);
    }

    pub fn get(&self, offset: usize) -> Option<ValueRef> {
        self.offsets.get(&offset).cloned()
    }
}