use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use ew_ordpack::{Decode, Encode};

#[derive(Encode, Decode, PartialEq, Debug)]
struct Registry {
    #[ordpack(sort)]
    entries: HashMap<String, u64>,
    #[ordpack(sort)]
    tags: HashSet<u32>,
}

fn build(keys: &[&str], reverse: bool) -> Registry {
    let mut entries = HashMap::new();
    let mut order: Vec<&str> = keys.to_vec();
    if reverse {
        order.reverse();
    }
    for key in order {
        entries.insert(key.to_string(), key.len() as u64);
    }

    let mut tags = HashSet::new();
    let mut tag_order: Vec<u32> = (0..16).collect();
    if reverse {
        tag_order.reverse();
    }
    for tag in tag_order {
        tags.insert(tag);
    }

    Registry { entries, tags }
}

#[test]
fn sorted_encoding_is_deterministic() {
    let keys = [
        "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta",
    ];
    let a = build(&keys, false);
    let b = build(&keys, true);

    // Different insertion order, so the hash iteration order differs; the
    // sorted encoder must still produce identical bytes.
    assert_eq!(
        ew_ordpack::to_vec(&a).unwrap(),
        ew_ordpack::to_vec(&b).unwrap()
    );
}

#[test]
fn sorted_roundtrip() {
    let keys = ["a", "bb", "ccc", "dddd"];
    let value = build(&keys, false);
    let bytes = ew_ordpack::to_vec(&value).unwrap();
    let decoded: Registry = ew_ordpack::from_slice(&bytes).unwrap();
    assert_eq!(decoded, value);
}

#[test]
fn sorted_btree_is_identity() {
    #[derive(Encode, Decode, PartialEq, Debug)]
    struct Ordered {
        #[ordpack(sort)]
        entries: BTreeMap<u8, u8>,
        #[ordpack(sort)]
        tags: BTreeSet<u8>,
    }

    let mut entries = BTreeMap::new();
    entries.insert(3, 30);
    entries.insert(1, 10);
    let mut tags = BTreeSet::new();
    tags.insert(9);
    tags.insert(2);

    let value = Ordered { entries, tags };
    let bytes = ew_ordpack::to_vec(&value).unwrap();
    assert_eq!(ew_ordpack::from_slice::<Ordered>(&bytes).unwrap(), value);
}
