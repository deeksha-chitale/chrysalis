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
fn harmony_has_all_keys() {
    let bytes = read_file();
    let val = parse(&bytes);
    let map = top_map(&val);

    let expected = [
        "byte_val", "int_val", "dbl_val", "undef_val", "yes_val", "no_val",
        "bytes_val", "lscalar_val", "utf8_val", "lutf8_val", "ver", "re", "code",
        "mixed_arr", "info_hash", "scalar_ref", "dog", "cat", "point",
        "num_a", "num_b", "config", "graph", "node_a", "weak_ovld",
        "tied_s", "tied_a", "tied_h", "sparse", "deep",
    ];

    for key in &expected {
        assert!(map.contains_key(key.as_bytes()), "missing key: {}", key);
    }
    println!("all {} expected keys present", expected.len());
}

#[test]
fn harmony_scalars() {
    let bytes = read_file();
    let val = parse(&bytes);
    let map = top_map(&val);

    // SX_BYTE
    assert!(matches!(*get(&map, "byte_val").borrow(), PerlValue::Integer(42)));

    // SX_INTEGER
    assert!(matches!(*get(&map, "int_val").borrow(), PerlValue::Integer(100_000)));

    // SX_DOUBLE
    match &*get(&map, "dbl_val").borrow() {
        PerlValue::Double(f) => assert!((*f - 2.718281828).abs() < 1e-9),
        other => panic!("dbl_val: {:?}", other),
    }

    // SX_UNDEF
    assert!(matches!(*get(&map, "undef_val").borrow(), PerlValue::Undef));

    // SX_SV_YES
    assert!(matches!(*get(&map, "yes_val").borrow(), PerlValue::Yes));

    // SX_SV_NO
    assert!(matches!(*get(&map, "no_val").borrow(), PerlValue::No));

    // SX_SCALAR — raw bytes containing 0xFF
    match &*get(&map, "bytes_val").borrow() {
        PerlValue::Bytes(b) => assert!(b.contains(&0xFF), "bytes_val should contain 0xFF"),
        other => panic!("bytes_val: {:?}", other),
    }

    // SX_LSCALAR — 300 bytes
    match &*get(&map, "lscalar_val").borrow() {
        PerlValue::Bytes(b) => assert_eq!(b.len(), 300),
        other => panic!("lscalar_val: {:?}", other),
    }

    // SX_UTF8STR — contains é
    match &*get(&map, "utf8_val").borrow() {
        PerlValue::String(s) => assert!(s.contains('é'), "utf8_val: {}", s),
        other => panic!("utf8_val: {:?}", other),
    }

    // SX_LUTF8STR — long utf8
    match &*get(&map, "lutf8_val").borrow() {
        PerlValue::String(s) => assert!(s.len() > 255, "lutf8_val len: {}", s.len()),
        other => panic!("lutf8_val: {:?}", other),
    }

    println!("scalars: ok");
}

#[test]
fn harmony_vstring() {
    let bytes = read_file();
    let val = parse(&bytes);
    let map = top_map(&val);

    // SX_VSTRING
    assert!(matches!(&*get(&map, "ver").borrow(), PerlValue::VString(_)),
        "ver should be VString, got {:?}", &*get(&map, "ver").borrow());
    println!("vstring: ok");
}

#[test]
fn harmony_regexp() {
    let bytes = read_file();
    let val = parse(&bytes);
    let map = top_map(&val);

    // SX_REGEXP — may be wrapped in Ref or Blessed depending on Perl version
    let re = get(&map, "re");
    let borrowed = re.borrow();
    match &*borrowed {
        PerlValue::Regexp { pattern, .. } => {
            match &*pattern.borrow() {
                PerlValue::Bytes(b) => {
                    let s = String::from_utf8_lossy(b);
                    assert!(s.contains("\\d"), "pattern should contain \\d: {}", s);
                }
                other => panic!("pattern: {:?}", other),
            }
            println!("regexp (direct): ok");
        }
        PerlValue::Ref(inner) => match &*inner.borrow() {
            PerlValue::Regexp { pattern, .. } => {
                match &*pattern.borrow() {
                    PerlValue::Bytes(b) => {
                        let s = String::from_utf8_lossy(b);
                        assert!(s.contains("\\d"), "pattern: {}", s);
                    }
                    other => panic!("pattern: {:?}", other),
                }
                println!("regexp (inside ref): ok");
            }
            PerlValue::Blessed(inner2, class) => {
                println!("regexp blessed as {}: ok", class);
            }
            other => panic!("re inner: {:?}", other),
        },
        PerlValue::Blessed(_, class) => println!("regexp blessed as {}: ok", class),
        other => panic!("re: {:?}", other),
    }
}

#[test]
fn harmony_code() {
    let bytes = read_file();
    let val = parse(&bytes);
    let map = top_map(&val);

    // SX_CODE
    match &*get(&map, "code").borrow() {
        PerlValue::Code(src) => {
            assert!(!src.is_empty(), "code source should not be empty");
            println!("code source ({} chars): {}...", src.len(), &src[..src.len().min(40)]);
        }
        other => panic!("code: {:?}", other),
    }
}

#[test]
fn harmony_blessed() {
    let bytes = read_file();
    let val = parse(&bytes);
    let map = top_map(&val);

    // SX_BLESS — first Animal
    match &*get(&map, "dog").borrow() {
        PerlValue::Blessed(inner, class) => {
            assert_eq!(class, "Animal");
            assert!(matches!(&*inner.borrow(), PerlValue::Hash(_)));
            println!("dog (SX_BLESS): Animal ok");
        }
        other => panic!("dog: {:?}", other),
    }

    // SX_IX_BLESS — second Animal (indexed)
    match &*get(&map, "cat").borrow() {
        PerlValue::Blessed(inner, class) => {
            assert_eq!(class, "Animal");
            assert!(matches!(&*inner.borrow(), PerlValue::Hash(_)));
            println!("cat (SX_IX_BLESS): Animal ok");
        }
        other => panic!("cat: {:?}", other),
    }
}

#[test]
fn harmony_hook() {
    let bytes = read_file();
    let val = parse(&bytes);
    let map = top_map(&val);

    // SX_HOOK — Point->STORABLE_freeze returns "3,4"
    match &*get(&map, "point").borrow() {
        PerlValue::Hook { class, frozen, .. } => {
            assert_eq!(class, "Point");
            assert_eq!(String::from_utf8_lossy(frozen), "3,4");
            println!("hook: Point frozen as \"3,4\" ok");
        }
        other => panic!("point: {:?}", other),
    }
}

#[test]
fn harmony_shared_identity() {
    let bytes = read_file();
    let val = parse(&bytes);
    let map = top_map(&val);

    // SX_OBJECT — graph = [$shared_node, $shared_node], both point to same Rc
    match &*get(&map, "graph").borrow() {
        PerlValue::Ref(inner) => match &*inner.borrow() {
            PerlValue::Array(items) => {
                assert_eq!(items.len(), 2);
                assert!(Rc::ptr_eq(&items[0], &items[1]),
                    "graph[0] and graph[1] should be the same Rc");
                println!("shared identity (SX_OBJECT): Rc::ptr_eq verified");
            }
            other => panic!("graph inner: {:?}", other),
        },
        other => panic!("graph: {:?}", other),
    }
}

#[test]
fn harmony_weak_ref() {
    let bytes = read_file();
    let val = parse(&bytes);
    let map = top_map(&val);

    // SX_WEAKREF — node_a->{peer} is weakened
    match &*get(&map, "node_a").borrow() {
        PerlValue::Hash(h) => {
            let peer = h.get(b"peer".as_slice()).expect("node_a.peer");
            assert!(matches!(&*peer.borrow(), PerlValue::WeakRef(_)),
                "node_a.peer should be WeakRef, got {:?}", &*peer.borrow());
            if let PerlValue::WeakRef(weak) = &*peer.borrow() {
                assert!(weak.upgrade().is_some(), "weak ref should still be alive");
            }
            println!("weak ref (SX_WEAKREF): ok");
        }
        other => panic!("node_a: {:?}", other),
    }
}

#[test]
fn harmony_flag_hash() {
    let bytes = read_file();
    let val = parse(&bytes);
    let map = top_map(&val);

    // SX_FLAG_HASH — config is a lock_keys restricted hash
    match &*get(&map, "config").borrow() {
        PerlValue::Ref(inner) => match &*inner.borrow() {
            PerlValue::FlagHash { entries, .. } => {
                assert!(entries.contains_key(b"host".as_slice()));
                assert!(entries.contains_key(b"port".as_slice()));
                println!("flag_hash (SX_FLAG_HASH): {} entries ok", entries.len());
            }
            other => panic!("config inner: {:?}", other),
        },
        other => panic!("config: {:?}", other),
    }
}

#[test]
fn harmony_tied() {
    let bytes = read_file();
    let val = parse(&bytes);
    let map = top_map(&val);

    // SX_TIED_SCALAR
    match &*get(&map, "tied_s").borrow() {
        PerlValue::Ref(inner) => assert!(
            matches!(&*inner.borrow(), PerlValue::TiedScalar(_)),
            "tied_s: {:?}", &*inner.borrow()
        ),
        other => panic!("tied_s: {:?}", other),
    }
    println!("tied_scalar (SX_TIED_SCALAR): ok");

    // SX_TIED_ARRAY
    match &*get(&map, "tied_a").borrow() {
        PerlValue::Ref(inner) => assert!(
            matches!(&*inner.borrow(), PerlValue::TiedArray(_)),
            "tied_a: {:?}", &*inner.borrow()
        ),
        other => panic!("tied_a: {:?}", other),
    }
    println!("tied_array (SX_TIED_ARRAY): ok");

    // SX_TIED_HASH
    match &*get(&map, "tied_h").borrow() {
        PerlValue::Ref(inner) => assert!(
            matches!(&*inner.borrow(), PerlValue::TiedHash(_)),
            "tied_h: {:?}", &*inner.borrow()
        ),
        other => panic!("tied_h: {:?}", other),
    }
    println!("tied_hash (SX_TIED_HASH): ok");
}

#[test]
fn harmony_sparse() {
    let bytes = read_file();
    let val = parse(&bytes);
    let map = top_map(&val);

    // SX_SV_UNDEF / SX_SVUNDEF_ELEM — sparse array
    match &*get(&map, "sparse").borrow() {
        PerlValue::Ref(inner) => match &*inner.borrow() {
            PerlValue::Array(items) => {
                assert_eq!(items.len(), 5, "sparse len");
                assert!(matches!(*items[1].borrow(), PerlValue::Undef), "sparse[1]");
                assert!(matches!(*items[3].borrow(), PerlValue::Undef), "sparse[3]");
                println!("sparse array (SX_SV_UNDEF): ok");
            }
            other => panic!("sparse inner: {:?}", other),
        },
        other => panic!("sparse: {:?}", other),
    }
}

#[test]
fn harmony_deep_lscalar() {
    let bytes = read_file();
    let val = parse(&bytes);
    let map = top_map(&val);

    // SX_LSCALAR in deeply nested hashes
    match &*get(&map, "deep").borrow() {
        PerlValue::Ref(r1) => match &*r1.borrow() {
            PerlValue::Hash(h1) => {
                let outer = h1.get(b"outer".as_slice()).expect("outer");
                match &*outer.borrow() {
                    PerlValue::Ref(r2) => match &*r2.borrow() {
                        PerlValue::Hash(h2) => {
                            let inner = h2.get(b"inner".as_slice()).expect("inner");
                            match &*inner.borrow() {
                                PerlValue::Ref(r3) => match &*r3.borrow() {
                                    PerlValue::Hash(h3) => {
                                        let blob = h3.get(b"blob".as_slice()).expect("blob");
                                        match &*blob.borrow() {
                                            PerlValue::Bytes(b) => {
                                                assert_eq!(b.len(), 300, "blob len");
                                                println!("deep lscalar (SX_LSCALAR): 300 bytes ok");
                                            }
                                            other => panic!("blob: {:?}", other),
                                        }
                                    }
                                    other => panic!("h3: {:?}", other),
                                },
                                other => panic!("r3: {:?}", other),
                            }
                        }
                        other => panic!("h2: {:?}", other),
                    },
                    other => panic!("r2: {:?}", other),
                }
            }
            other => panic!("h1: {:?}", other),
        },
        other => panic!("deep: {:?}", other),
    }
}