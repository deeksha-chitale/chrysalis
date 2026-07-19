use std::collections::HashMap;
use crate::storable::value::{ValueRef}; 

pub struct SerealSeenTable {
    offsets: HashMap<usize, ValueRef>,
    class_offsets: HashMap<usize, String>,
}

impl SerealSeenTable {
    pub fn new() -> Self {
        Self { offsets: HashMap::new(), class_offsets: HashMap::new() }
    }

    pub fn register(&mut self, offset: usize, value: ValueRef) {
        self.offsets.insert(offset, value);
    }

    pub fn get(&self, offset: usize) -> Option<ValueRef> {
        self.offsets.get(&offset).cloned()
    }

    pub fn register_class(&mut self, offset: usize, class: String) {
        self.class_offsets.insert(offset, class);
    }

    pub fn get_class(&self, offset: usize) -> Option<String> {
        self.class_offsets.get(&offset).cloned()
    }
}
