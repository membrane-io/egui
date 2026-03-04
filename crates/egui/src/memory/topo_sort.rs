use ahash::{HashMap, HashSet};

use crate::LayerId;

// MEMBRANE: stable topological sort of sublayers.
//
// Reorders `order` so that every parent layer appears before its children, while preserving the
// relative order of unrelated layers. Children are grouped immediately after their parent.
//
// This adds support for sublayer parents who are also sublayers and fixes a flickering issue in
// the dashboard where stack blocks are sublayers of the backdrop but the pick overlays are
// sublayers of the block.
//
// Circular dependencies (A sublayer of B, B sublayer of A) are detected and left in place.
//
// Based on: <https://blog.gapotchenko.com/stable-topological-sort> but instead of moving the
// parent back, it moves the children forward so if the parent was moved to the top, it stays
// on top (except for its sublayers).
pub fn stable_topological_sort_sublayers(
    order: &mut Vec<LayerId>,
    sublayers: HashMap<LayerId, HashSet<LayerId>>,
) {
    if sublayers.is_empty() {
        return;
    }

    let mut parents: HashMap<LayerId, LayerId> = HashMap::default();
    let mut parent_ids: HashSet<LayerId> = HashSet::default();
    for (parent, children) in sublayers {
        parent_ids.insert(parent);
        for child in children {
            parents.insert(child, parent);
        }
    }

    if parents.is_empty() {
        return;
    }

    // Quick check: is every child already after its parent? O(n) scan, O(p) hash ops.
    // On most frames the order is already correct, so we skip all further work.
    {
        let mut seen_parents: HashSet<LayerId> = HashSet::default();
        let mut already_sorted = true;
        for &layer in order.iter() {
            if parent_ids.contains(&layer) {
                seen_parents.insert(layer);
            }
            if let Some(&parent) = parents.get(&layer) {
                if !seen_parents.contains(&parent) {
                    already_sorted = false;
                    break;
                }
            }
        }
        if already_sorted {
            return;
        }
    }

    fn is_ancestor(parents: &HashMap<LayerId, LayerId>, child: LayerId, ancestor: LayerId) -> bool {
        let mut current = child;
        while let Some(parent) = parents.get(&current) {
            if parent == &ancestor {
                return true;
            }
            current = *parent;
        }
        false
    }

    // Build position map for participating layers only. O(n) scan, O(p) storage.
    let mut pos_of: HashMap<LayerId, usize> = HashMap::default();
    for (i, &layer) in order.iter().enumerate() {
        if parents.contains_key(&layer) || parent_ids.contains(&layer) {
            pos_of.insert(layer, i);
        }
    }

    let mut children_of: HashMap<LayerId, Vec<(usize, LayerId)>> = HashMap::default();
    let mut is_child = vec![false; order.len()];

    for (&child, &parent) in &parents {
        if let (Some(&cp), Some(_)) = (pos_of.get(&child), pos_of.get(&parent)) {
            if !is_ancestor(&parents, parent, child) {
                children_of.entry(parent).or_default().push((cp, child));
                is_child[cp] = true;
            }
        }
    }

    for children in children_of.values_mut() {
        children.sort_by_key(|(pos, _)| *pos);
    }

    let mut result = Vec::with_capacity(order.len());
    let mut emitted = vec![false; order.len()];

    fn emit_tree(
        idx: usize,
        node: LayerId,
        result: &mut Vec<LayerId>,
        emitted: &mut [bool],
        children_of: &HashMap<LayerId, Vec<(usize, LayerId)>>,
    ) {
        if emitted[idx] {
            return;
        }
        emitted[idx] = true;
        result.push(node);
        if let Some(children) = children_of.get(&node) {
            for &(child_idx, child) in children {
                emit_tree(child_idx, child, result, emitted, children_of);
            }
        }
    }

    for idx in 0..order.len() {
        if !emitted[idx] && !is_child[idx] {
            emit_tree(idx, order[idx], &mut result, &mut emitted, &children_of);
        }
    }

    for idx in 0..order.len() {
        if !emitted[idx] {
            emit_tree(idx, order[idx], &mut result, &mut emitted, &children_of);
        }
    }

    *order = result;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Id, Order};

    fn l(name: &str) -> LayerId {
        LayerId::new(Order::Middle, Id::new(name))
    }

    fn ids(names: &[&str]) -> Vec<LayerId> {
        names.iter().map(|n| l(n)).collect()
    }

    fn sort(order: &[&str], pairs: &[(&str, &str)]) -> Vec<String> {
        let mut order = ids(order);
        let mut sublayers: HashMap<LayerId, HashSet<LayerId>> = HashMap::default();
        for &(parent, child) in pairs {
            sublayers.entry(l(parent)).or_default().insert(l(child));
        }
        stable_topological_sort_sublayers(&mut order, sublayers);
        order.iter().map(|id| format!("{id:?}")).collect()
    }

    fn expect(names: &[&str]) -> Vec<String> {
        ids(names).iter().map(|id| format!("{id:?}")).collect()
    }

    #[test]
    fn no_sublayers() {
        assert_eq!(sort(&["a", "b", "c"], &[]), expect(&["a", "b", "c"]));
    }

    #[test]
    fn already_correct() {
        assert_eq!(sort(&["p", "c"], &[("p", "c")]), expect(&["p", "c"]));
    }

    #[test]
    fn child_before_parent() {
        assert_eq!(sort(&["c", "p"], &[("p", "c")]), expect(&["p", "c"]));
    }

    #[test]
    fn child_before_parent_with_unrelated() {
        assert_eq!(
            sort(&["c", "x", "p", "y"], &[("p", "c")]),
            expect(&["x", "p", "c", "y"])
        );
    }

    #[test]
    fn multiple_children_before_parent() {
        assert_eq!(
            sort(&["c1", "c2", "x", "p"], &[("p", "c1"), ("p", "c2")]),
            expect(&["x", "p", "c1", "c2"])
        );
    }

    #[test]
    fn children_preserve_relative_order() {
        assert_eq!(
            sort(&["c2", "c1", "p"], &[("p", "c1"), ("p", "c2")]),
            expect(&["p", "c2", "c1"])
        );
    }

    #[test]
    fn deep_hierarchy_all_reversed() {
        assert_eq!(
            sort(&["g", "c", "p"], &[("p", "c"), ("c", "g")]),
            expect(&["p", "c", "g"])
        );
    }

    #[test]
    fn deep_hierarchy_already_correct() {
        assert_eq!(
            sort(&["p", "c", "g"], &[("p", "c"), ("c", "g")]),
            expect(&["p", "c", "g"])
        );
    }

    #[test]
    fn dashboard_scenario() {
        assert_eq!(
            sort(
                &["pick", "block", "other", "backdrop"],
                &[("backdrop", "block"), ("block", "pick")]
            ),
            expect(&["other", "backdrop", "block", "pick"])
        );
    }

    #[test]
    fn circular_dependency_left_in_place() {
        assert_eq!(
            sort(&["a", "b", "x"], &[("a", "b"), ("b", "a")]),
            expect(&["a", "b", "x"])
        );
    }

    #[test]
    fn parent_not_in_order() {
        assert_eq!(
            sort(&["c", "x"], &[("not_present", "c")]),
            expect(&["c", "x"])
        );
    }

    #[test]
    fn child_not_in_order() {
        assert_eq!(
            sort(&["p", "x"], &[("p", "not_present")]),
            expect(&["p", "x"])
        );
    }

    #[test]
    fn independent_groups() {
        assert_eq!(
            sort(&["c1", "c2", "p1", "p2"], &[("p1", "c1"), ("p2", "c2")]),
            expect(&["p1", "c1", "p2", "c2"])
        );
    }

    #[test]
    fn preserves_length() {
        let result = sort(&["a", "b", "c", "d", "e"], &[("c", "a"), ("c", "b")]);
        assert_eq!(result.len(), 5);
    }

    #[test]
    fn many_layers_no_sublayers() {
        let names: Vec<String> = (0..100).map(|i| format!("l{i}")).collect();
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        assert_eq!(sort(&refs, &[]), expect(&refs));
    }

    #[test]
    fn parent_with_many_children() {
        let children: Vec<String> = (0..20).map(|i| format!("c{i}")).collect();
        let mut all: Vec<&str> = children.iter().map(|s| s.as_str()).collect();
        all.push("p");
        let pairs: Vec<(&str, &str)> = children.iter().map(|c| ("p", c.as_str())).collect();
        let result = sort(&all, &pairs);
        let p_pos = result.iter().position(|x| *x == expect(&["p"])[0]).unwrap();
        for c in &children {
            let c_pos = result
                .iter()
                .position(|x| *x == expect(&[c.as_str()])[0])
                .unwrap();
            assert!(p_pos < c_pos, "parent should be before {c}");
        }
    }
}
