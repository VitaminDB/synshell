//! Порядок исполнения графа (алгоритм Кана). Общий для всех движков:
//! ноды и рёбра — любые копируемые ключи.

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;

/// Топологический порядок: ноды без входящих рёбер — первыми, порядок
/// среди равных — как во входном срезе. Второй элемент — ноды в цикле
/// (до них алгоритм не дошёл), в порядке входного среза.
pub fn topo<K: Copy + Eq + Hash>(nodes: &[K], edges: impl IntoIterator<Item = (K, K)>) -> (Vec<K>, Vec<K>) {
    let known: HashSet<K> = nodes.iter().copied().collect();
    let mut in_deg: HashMap<K, usize> = nodes.iter().map(|&n| (n, 0)).collect();
    let mut out: HashMap<K, Vec<K>> = HashMap::new();
    let mut seen: HashSet<(K, K)> = HashSet::new();
    for (a, b) in edges {
        if a == b || !known.contains(&a) || !known.contains(&b) || !seen.insert((a, b)) {
            continue;
        }
        *in_deg.get_mut(&b).unwrap() += 1;
        out.entry(a).or_default().push(b);
    }
    let mut queue: VecDeque<K> = nodes.iter().copied().filter(|n| in_deg[n] == 0).collect();
    let mut order = Vec::with_capacity(nodes.len());
    while let Some(n) = queue.pop_front() {
        order.push(n);
        for &m in out.get(&n).map(Vec::as_slice).unwrap_or(&[]) {
            let d = in_deg.get_mut(&m).unwrap();
            *d -= 1;
            if *d == 0 {
                queue.push_back(m);
            }
        }
    }
    let done: HashSet<K> = order.iter().copied().collect();
    let cyclic = nodes.iter().copied().filter(|n| !done.contains(n)).collect();
    (order, cyclic)
}

/// Предки ноды (транзитивно) по рёбрам.
pub fn ancestors<K: Copy + Eq + Hash>(node: K, edges: &[(K, K)]) -> HashSet<K> {
    let mut back: HashMap<K, Vec<K>> = HashMap::new();
    for &(a, b) in edges {
        back.entry(b).or_default().push(a);
    }
    let mut seen = HashSet::new();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        for &p in back.get(&n).map(Vec::as_slice).unwrap_or(&[]) {
            if seen.insert(p) {
                stack.push(p);
            }
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_and_cycle() {
        let (o, c) = topo(&[1, 2, 3, 4], [(3, 1), (1, 2)]);
        assert_eq!(o, vec![3, 4, 1, 2]);
        assert!(c.is_empty());
        let (o, c) = topo(&[1, 2, 3], [(1, 2), (2, 1), (2, 3)]);
        assert!(o.is_empty());
        assert_eq!(c, vec![1, 2, 3]);
        assert_eq!(ancestors(2, &[(3, 1), (1, 2), (4, 5)]), [1, 3].into_iter().collect());
    }
}
