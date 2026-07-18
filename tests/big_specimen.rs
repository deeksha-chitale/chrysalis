use chrysalis::storable::{header, body};
use chrysalis::storable::value::{PerlValue, SeenTable, ClassTable};

#[test]
fn parses_big_specimen() {
    let bytes = std::fs::read("big_specimen.bin")
        .expect("run tools/big_specimen.pl first");

    let (h, remaining) = header::parse(&bytes).expect("header should parse");
    let config = h.body_config();
    println!("header: {:?}", h);
    println!("body starts with: {:02x?}", &remaining[..remaining.len().min(256)]);
    println!("total body length: {}", remaining.len());
    println!("bytes 550 onward: {:02x?}", &remaining[550..]);
    
    let mut cursor = body::Cursor::new(remaining);
    let mut seen = SeenTable::new();
    let mut classes = ClassTable::new();
    let result = body::read_value(&mut cursor, &mut seen, &mut classes, &config);
    if let Err(e) = &result {
        let pos = cursor.pos();
        println!("failed at body position {} of {}, error: {:?}", pos, remaining.len(), e);
        println!("seen table has {} entries at time of failure", seen.len());
        let start = pos.saturating_sub(32);
        let end = (pos + 32).min(remaining.len());
        println!("bytes around failure (pos {}-{}): {:02x?}", start, end, &remaining[start..end]);
    }
    let val = result.expect("body should parse");
    let expected_keys: [&[u8]; 7] = [
        b"scalars", b"blessed", b"shared", b"cyclic", b"regex", b"vstring", b"nested",
    ];

    match &*val.borrow() {
        PerlValue::Ref(inner) => match &*inner.borrow() {
            PerlValue::Hash(map) => {
                for key in &expected_keys {
                    assert!(
                        map.contains_key(*key),
                        "missing key: {:?}",
                        std::str::from_utf8(*key)
                    );
                }
            }
            other => panic!("expected Hash inside Ref, got {:?}", other),
        },
        PerlValue::Hash(map) => {
            for key in &expected_keys {
                assert!(
                    map.contains_key(*key),
                    "missing key: {:?}",
                    std::str::from_utf8(*key)
                );
            }
        }
        other => panic!("expected top-level Hash or Ref, got {:?}", other),
    }

    println!("registered {} values in seen table", seen.len());
}