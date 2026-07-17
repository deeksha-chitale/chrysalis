use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

/// Shorthand — a shared, mutable PerlValue.
pub type ValueRef = Rc<RefCell<PerlValue>>;

#[derive(Debug, Clone)]
pub enum PerlValue {
    // Perl's three immortal singletons
    Undef,
    Yes,
    No,

    // Scalars
    Integer(i64),
    Double(f64),
    Bytes(Vec<u8>),
    String(String),

    // Containers — children are shared refs so multiple parents can point at the same child
    Array(Vec<ValueRef>),
    Hash(HashMap<Vec<u8>, ValueRef>),

    // References
    Ref(ValueRef),
    WeakRef(Weak<RefCell<PerlValue>>),

    // A blessed value: inner data + class name
    Blessed(ValueRef, String),
}

impl PerlValue {
    /// Wrap into a shared, mutable ref — the standard way to build one.
    pub fn wrap(value: PerlValue) -> ValueRef {
        Rc::new(RefCell::new(value))
    }
}

/// A table of values indexed by the order they were parsed.
/// Used to resolve SX_OBJECT back-references and preserve shared structure.
pub struct SeenTable {
    values: Vec<ValueRef>,
}

impl SeenTable {
    pub fn new() -> Self {
        Self { values: Vec::new() }
    }

    /// Register a value and return its index.
    pub fn register(&mut self, value: ValueRef) -> usize {
        let index = self.values.len();
        self.values.push(value);
        index
    }

    /// Look up a value by index. Returns None if index is out of range.
    pub fn get(&self, index: usize) -> Option<ValueRef> {
        self.values.get(index).cloned()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }
}

impl Default for SeenTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_returns_sequential_indices() {
        let mut table = SeenTable::new();
        let a = PerlValue::wrap(PerlValue::Integer(1));
        let b = PerlValue::wrap(PerlValue::Integer(2));
        assert_eq!(table.register(a), 0);
        assert_eq!(table.register(b), 1);
    }

    #[test]
    fn get_returns_the_registered_value() {
        let mut table = SeenTable::new();
        let val = PerlValue::wrap(PerlValue::Integer(42));
        let idx = table.register(val.clone());
        let fetched = table.get(idx).unwrap();
        assert!(Rc::ptr_eq(&val, &fetched));  // same Rc, not a copy
    }

    #[test]
    fn get_out_of_range_returns_none() {
        let table = SeenTable::new();
        assert!(table.get(0).is_none());
    }

    #[test]
    fn shared_registration_preserves_identity() {
        // Register the same value twice — both indices point to the same Rc.
        let mut table = SeenTable::new();
        let val = PerlValue::wrap(PerlValue::Integer(42));
        let i = table.register(val.clone());
        let j = table.register(val.clone());
        assert!(Rc::ptr_eq(&table.get(i).unwrap(), &table.get(j).unwrap()));
    }
}

pub struct ClassTable {
    names: Vec<String>,
}

impl ClassTable {
    pub fn new() -> Self {
        Self { names: Vec::new() }
    }

    pub fn register(&mut self, name: String) -> usize {
        let index = self.names.len();
        self.names.push(name);
        index
    }

    pub fn get(&self, index: usize) -> Option<&str> {
        self.names.get(index).map(|s| s.as_str())
    }
}

impl Default for ClassTable {
    fn default() -> Self {
        Self::new()
    }
}