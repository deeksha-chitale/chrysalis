//! Integration test for harmony.bin — exercises every Storable SX tag.
//! Run `perl tools/harmony.pl` first to generate harmony.bin.

use chrysalis::storable::{header, body};
use chrysalis::storable::value::{ClassTable, PerlValue, SeenTable, ValueRef};
use std::collections::HashMap;
use std::rc::Rc;

// ─── helpers ──────────────────────────────────────────────────────────────────

fn parse(bytes: &[u8]) -> ValueRef {
    let (h, remaining) = header::parse(bytes).expect("header should parse");
    let config = h.body_config();
    let mut cursor = body::Cursor::new(remaining);
    let mut seen = SeenTable::new();
    let mut classes = ClassTable::new();
    body::read_value(&mut cursor, &mut seen, &mut classes, &config)
        .expect("body should parse")
}

fn read_file() -> Vec<u8> {
    std::fs::read("harmony.bin").expect("run `perl tools/harmony.pl` first")
}

fn get(map: &HashMap<Vec<u8>, ValueRef>, key: &str) -> ValueRef {
    map.get(key.as_bytes())
        .unwrap_or_else(|| panic!("missing key: {}", key))
        .clone()
}

fn top_map(val: &ValueRef) -> HashMap<Vec<u8>, ValueRef> {
    match &*val.borrow() {
        PerlValue::Hash(m) => m.clone(),
        other => panic!("expected top-level Hash, got {:?}", other),
    }
}

// ─── tests ────────────────────────────────────────────────────────────────────

#[test]
fn harmony_parses_completely() {
    let bytes = read_file();
    let (h, remaining) = header::parse(&bytes).expect("header");
    let config = h.body_config();
    let mut cursor = body::Cursor::new(remaining);
    let mut seen = SeenTable::new();
    let mut classes = ClassTable::new();
    let result = body::read_value(&mut cursor, &mut seen, &mut classes, &config);
    if let Err(e) = &result {
        let pos = cursor.pos();
        println!("failed at pos {} of {}: {:?}", pos, remaining.len(), e);
        let start = pos.saturating_sub(16);
        let end = (pos + 8).min(remaining.len());
        println!("bytes: {:02x?}", &remaining[start..end]);
        panic!("parse failed: {:?}", e);
    }
    println!("parsed ok, seen {} entries", seen.len());
}

#[test]
fn harmony_code_fix() {
    let bytes = read_file();
    let val = parse(&bytes);
    let map = top_map(&val);
    let code = get(&map, "code");
    let borrowed = code.borrow();
    match &*borrowed {
        PerlValue::Code(src) => assert!(!src.is_empty()),
        PerlValue::Ref(inner) => match &*inner.borrow() {
            PerlValue::Code(src) => assert!(!src.is_empty()),
            other => panic!("code inner: {:?}", other),
        },
        other => panic!("code: {:?}", other),
    }
}

#[test]
fn harmony_hook_fix() {
    let bytes = read_file();
    let val = parse(&bytes);
    let map = top_map(&val);
    let point = get(&map, "point");
    let borrowed = point.borrow();
    let check = |v: &PerlValue| match v {
        PerlValue::Hook { class, frozen, .. } => {
            assert_eq!(class, "Point");
            assert_eq!(String::from_utf8_lossy(frozen), "3,4");
        }
        other => panic!("point: {:?}", other),
    };
    match &*borrowed {
        PerlValue::Ref(inner) => check(&inner.borrow()),
        other => check(other),
    }
}

#[test]
fn harmony_shared_identity_fix() {
    let bytes = read_file();
    let val = parse(&bytes);
    let map = top_map(&val);
    let graph = get(&map, "graph");
    let borrowed = graph.borrow();
    let items = match &*borrowed {
        PerlValue::Array(items) => items.clone(),
        PerlValue::Ref(inner) => match &*inner.borrow() {
            PerlValue::Array(items) => items.clone(),
            other => panic!("graph inner: {:?}", other),
        },
        other => panic!("graph: {:?}", other),
    };
    assert_eq!(items.len(), 2);

    // Each element is itself a Ref; unwrap one level to compare the shared hash
    let inner0 = match &*items[0].borrow() {
        PerlValue::Ref(h) => h.clone(),
        other => panic!("graph[0]: {:?}", other),
    };
    let inner1 = match &*items[1].borrow() {
        PerlValue::Ref(h) => h.clone(),
        other => panic!("graph[1]: {:?}", other),
    };
    assert!(Rc::ptr_eq(&inner0, &inner1),
        "the two refs should point at the same underlying hash");
}

#[test]
fn harmony_weak_ref_fix() {
    let bytes = read_file();
    let val = parse(&bytes);
    let map = top_map(&val);
    let node_a = get(&map, "node_a");
    let borrowed = node_a.borrow();
    let hash = match &*borrowed {
        PerlValue::Hash(h) => h.clone(),
        PerlValue::Ref(r) => match &*r.borrow() {
            PerlValue::Hash(h) => h.clone(),
            other => panic!("node_a inner: {:?}", other),
        },
        other => panic!("node_a: {:?}", other),
    };
    let peer = hash.get(b"peer".as_slice()).expect("peer").clone();
    assert!(matches!(&*peer.borrow(), PerlValue::WeakRef(_)));
}

#[test]
fn harmony_blessed_fix() {
    let bytes = read_file();
    let val = parse(&bytes);
    let map = top_map(&val);
    let unwrap = |v: &ValueRef| -> String {
        let b = v.borrow();
        match &*b {
            PerlValue::Blessed(_, c) => c.clone(),
            PerlValue::Ref(r) => {
                let rb = r.borrow();
                match &*rb {
                    PerlValue::Blessed(_, c) => c.clone(),
                    other => panic!("expected Blessed: {:?}", other),
                }
            }
            other => panic!("expected Blessed/Ref: {:?}", other),
        }
    };
    assert_eq!(unwrap(&get(&map, "dog")), "Animal");
    assert_eq!(unwrap(&get(&map, "cat")), "Animal");
}