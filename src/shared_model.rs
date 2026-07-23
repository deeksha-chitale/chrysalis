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

    Overloaded(ValueRef),
    WeakOverloaded(Weak<RefCell<PerlValue>>),
    VString(Vec<u8>),

    FlagHash {
        hash_flags: u8,
        entries: HashMap<Vec<u8>, (u8, ValueRef)>,
    },

    TiedScalar(ValueRef),
    TiedArray(ValueRef),
    TiedHash(ValueRef),
    TiedKey(ValueRef, ValueRef),        // (tied object, key)
    TiedIdx(ValueRef, i64),             // (tied object, index)

    Code(String),  // Perl source of a code ref

    Regexp { pattern: ValueRef, flags: String },

    Hook {
        class: String,
        obj_type: u8,           // 0=scalar, 1=array, 2=hash, 3=extra
        frozen: Vec<u8>,
        refs: Vec<ValueRef>,    // resolved from seen-table indices
        recurse: Vec<ValueRef>, // values consumed by SHF_NEED_RECURSE (usually empty)
    },

    UnsignedInteger(u64),
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

/// A table of class names indexed by the order they were parsed.
/// Used to resolve SX_IX_BLESS (and OBJECTV-style) class-by-index back-references.
pub struct ClassTable {
    names: Vec<String>,
}

impl ClassTable {
    pub fn new() -> Self {
        Self { names: Vec::new() }
    }

    /// Register a class name and return its index.
    pub fn register(&mut self, name: String) -> usize {
        let index = self.names.len();
        self.names.push(name);
        index
    }

    /// Look up a class name by index. Returns None if index is out of range.
    pub fn get(&self, index: usize) -> Option<&str> {
        self.names.get(index).map(|s| s.as_str())
    }
}

impl Default for ClassTable {
    fn default() -> Self {
        Self::new()
    }
}
