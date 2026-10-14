//! Scene/TweenCollectionSpec → Rust 集成测试
//!
//! 原始：25 it() → 11 A 类（14 C 类：throws/callbacks-spy）
//! A 类：add(2) + add_zero_duration(1) + cancelTween(1) + remove(1) +
//!          removeAll(1) + get(1) + update(1) + addProperty(1) + addAlpha(1) + addOffsetIncrement(1)

use cesium_animation::tween::{EasingFunction, TweenCollection, TweenOptions};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

fn make_start_stop(start_val: f64, stop_val: f64) -> (HashMap<String, f64>, HashMap<String, f64>) {
    let mut s = HashMap::new();
    s.insert("value".to_string(), start_val);
    let mut e = HashMap::new();
    e.insert("value".to_string(), stop_val);
    (s, e)
}

#[test]
fn test_add_adds_a_tween() {
    let (start, stop) = make_start_stop(0.0, 1.0);
    let mut tweens = TweenCollection::new();
    let mut opts = TweenOptions::new(start.clone(), stop.clone(), 1.0);
    opts.delay = 0.5;
    opts.easing_function = EasingFunction::QuadraticIn;
    let idx = tweens.add(opts).unwrap();

    let tween = tweens.get(idx).unwrap();
    assert_eq!(tween.start_object().get("value").unwrap(), &0.0);
    assert_eq!(tween.stop_object().get("value").unwrap(), &1.0);
    assert_eq!(tween.duration(), 1.0);
    assert_eq!(tween.delay(), 0.5);
    assert_eq!(tween.easing_function(), EasingFunction::QuadraticIn);
}

#[test]
fn test_add_adds_a_tween_with_defaults() {
    let (start, stop) = make_start_stop(0.0, 1.0);
    let mut tweens = TweenCollection::new();
    let opts = TweenOptions::new(start, stop, 1.0);
    let idx = tweens.add(opts).unwrap();

    let tween = tweens.get(idx).unwrap();
    assert_eq!(tween.start_object().get("value").unwrap(), &0.0);
    assert_eq!(tween.stop_object().get("value").unwrap(), &1.0);
    assert_eq!(tween.duration(), 1.0);
    assert_eq!(tween.delay(), 0.0);
    assert_eq!(tween.easing_function(), EasingFunction::LinearNone);
}

#[test]
fn test_add_with_duration_of_zero() {
    let mut tweens = TweenCollection::new();
    let completed = Rc::new(RefCell::new(false));
    let c = completed.clone();

    let (start, stop) = make_start_stop(0.0, 1.0);
    let mut opts = TweenOptions::new(start, stop, 0.0);
    opts.complete = Some(Box::new(move || { *c.borrow_mut() = true; }));
    let result = tweens.add(opts);

    assert_eq!(tweens.len(), 0);
    assert!(result.is_none());
    assert!(*completed.borrow());
}

#[test]
fn test_cancel_tween_cancels() {
    let mut tweens = TweenCollection::new();
    let cancelled = Rc::new(RefCell::new(false));
    let c = cancelled.clone();

    let (start, stop) = make_start_stop(0.0, 1.0);
    let mut opts = TweenOptions::new(start, stop, 1.0);
    opts.cancel = Some(Box::new(move || { *c.borrow_mut() = true; }));
    let idx = tweens.add(opts).unwrap();

    assert_eq!(tweens.len(), 1);
    tweens.cancel_tween(idx);
    assert!(*cancelled.borrow());
    assert_eq!(tweens.len(), 0);
}

#[test]
fn test_remove_removes_a_tween() {
    let mut tweens = TweenCollection::new();
    let cancelled = Rc::new(RefCell::new(false));
    let c = cancelled.clone();

    let (start, stop) = make_start_stop(0.0, 1.0);
    let mut opts = TweenOptions::new(start, stop, 1.0);
    opts.cancel = Some(Box::new(move || { *c.borrow_mut() = true; }));
    let idx = tweens.add(opts).unwrap();

    assert_eq!(tweens.len(), 1);
    assert!(tweens.contains(idx));

    let removed = tweens.remove(idx);
    assert!(removed);
    assert_eq!(tweens.len(), 0);
    assert!(!tweens.contains(idx));
    assert!(*cancelled.borrow());

    // 再次移除返回 false
    let removed_again = tweens.remove(idx);
    assert!(!removed_again);
}

#[test]
fn test_remove_all_removes_all() {
    let mut tweens = TweenCollection::new();
    let cancel_count = Rc::new(RefCell::new(0));

    for _ in 0..2 {
        let c = cancel_count.clone();
        let (start, stop) = make_start_stop(0.0, 1.0);
        let mut opts = TweenOptions::new(start, stop, 1.0);
        opts.cancel = Some(Box::new(move || { *c.borrow_mut() += 1; }));
        tweens.add(opts);
    }

    assert_eq!(tweens.len(), 2);
    tweens.remove_all();
    assert_eq!(tweens.len(), 0);
    assert_eq!(*cancel_count.borrow(), 2);
}

#[test]
fn test_get_returns_a_tween() {
    let mut tweens = TweenCollection::new();
    let (s1, e1) = make_start_stop(0.0, 1.0);
    let (s2, e2) = make_start_stop(2.0, 3.0);
    tweens.add(TweenOptions::new(s1, e1, 1.0));
    tweens.add(TweenOptions::new(s2, e2, 1.0));

    assert_eq!(tweens.get(0).unwrap().start_object().get("value").unwrap(), &0.0);
    assert_eq!(tweens.get(1).unwrap().start_object().get("value").unwrap(), &2.0);
}

#[test]
fn test_update_animates_a_tween() {
    let mut tweens = TweenCollection::new();
    let update_values = Rc::new(RefCell::new(Vec::new()));
    let completed = Rc::new(RefCell::new(false));

    let u = update_values.clone();
    let c = completed.clone();
    let (start, stop) = make_start_stop(0.0, 1.0);
    let mut opts = TweenOptions::new(start, stop, 1.0);
    opts.update = Some(Box::new(move |values: &HashMap<String, f64>| {
        u.borrow_mut().push(*values.get("value").unwrap());
    }));
    opts.complete = Some(Box::new(move || { *c.borrow_mut() = true; }));
    tweens.add(opts);

    assert_eq!(tweens.len(), 1);

    tweens.update(0.0);
    assert_eq!(update_values.borrow().last().unwrap(), &0.0);

    tweens.update(0.5);
    assert_eq!(update_values.borrow().last().unwrap(), &0.5);

    tweens.update(1.0);
    assert_eq!(update_values.borrow().last().unwrap(), &1.0);

    assert!(*completed.borrow());
    assert_eq!(tweens.len(), 0);
}

#[test]
fn test_update_animates_add_property() {
    let mut tweens = TweenCollection::new();
    let object = Rc::new(RefCell::new({
        let mut m = HashMap::new();
        m.insert("property".to_string(), 0.0);
        m
    }));

    tweens.add_property(0.0, 1.0, 1.0, 0.0, EasingFunction::LinearNone, object.clone(), "property".to_string());
    tweens.update(0.0);
    tweens.update(0.5);
    assert!((object.borrow().get("property").unwrap() - 0.5).abs() < 1e-10);
}

#[test]
fn test_update_animates_add_alpha() {
    let mut tweens = TweenCollection::new();
    let uniforms = Rc::new(RefCell::new({
        let mut m = HashMap::new();
        m.insert("lightColor.alpha".to_string(), 1.0);
        m.insert("darkColor.alpha".to_string(), 1.0);
        m
    }));

    tweens.add_alpha(
        1.0, 0.0, 1.0,
        uniforms.clone(),
        vec!["lightColor".to_string(), "darkColor".to_string()],
    );
    tweens.update(0.0);
    tweens.update(0.5);
    assert!((uniforms.borrow().get("lightColor.alpha").unwrap() - 0.5).abs() < 1e-10);
    assert!((uniforms.borrow().get("darkColor.alpha").unwrap() - 0.5).abs() < 1e-10);
}

#[test]
fn test_update_animates_add_offset_increment() {
    let mut tweens = TweenCollection::new();
    let uniforms = Rc::new(RefCell::new({
        let mut m = HashMap::new();
        m.insert("offset".to_string(), 0.0);
        m
    }));

    tweens.add_offset_increment(1.0, uniforms.clone());
    tweens.update(0.0);
    tweens.update(0.5);
    assert!((uniforms.borrow().get("offset").unwrap() - 0.5).abs() < 1e-10);
}
