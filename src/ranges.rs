//! A set of byte positions, kept as sorted separate half-open ranges [start, end).

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RangeSet {
    items: Vec<(u64, u64)>,
}

impl RangeSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_items(items: &[(u64, u64)]) -> Self {
        let mut s = Self::new();
        for (a, b) in items {
            s.add(*a, *b);
        }
        s
    }

    pub fn items(&self) -> &[(u64, u64)] {
        &self.items
    }

    pub fn add(&mut self, start: u64, end: u64) {
        if start >= end {
            return;
        }
        let at = self.items.partition_point(|r| r.1 < start);
        let mut merged = (start, end);
        let mut stop = at;
        while stop < self.items.len() && self.items[stop].0 <= merged.1 {
            merged.0 = merged.0.min(self.items[stop].0);
            merged.1 = merged.1.max(self.items[stop].1);
            stop += 1;
        }
        self.items.splice(at..stop, [merged]);
    }

    /// How many bytes are in the set.
    pub fn len(&self) -> u64 {
        self.items.iter().map(|r| r.1 - r.0).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn covers(&self, start: u64, end: u64) -> bool {
        if start >= end {
            return true;
        }
        let at = self.items.partition_point(|r| r.1 <= start);
        self.items.get(at).map(|r| r.0 <= start && r.1 >= end).unwrap_or(false)
    }

    /// The first stretch of [0, total) that isn't in the set.
    pub fn first_gap(&self, total: u64) -> Option<(u64, u64)> {
        let mut pos = 0;
        for r in &self.items {
            if r.0 > pos {
                return Some((pos, r.0.min(total)));
            }
            pos = r.1;
            if pos >= total {
                return None;
            }
        }
        (pos < total).then_some((pos, total))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_merges_touching_and_overlapping_ranges() {
        let mut s = RangeSet::new();
        s.add(10, 20);
        s.add(30, 40);
        assert_eq!(s.items(), &[(10, 20), (30, 40)]);
        s.add(20, 25);
        assert_eq!(s.items(), &[(10, 25), (30, 40)]);
        s.add(5, 35);
        assert_eq!(s.items(), &[(5, 40)]);
        s.add(0, 1);
        s.add(50, 60);
        assert_eq!(s.items(), &[(0, 1), (5, 40), (50, 60)]);
        s.add(7, 8);
        assert_eq!(s.items(), &[(0, 1), (5, 40), (50, 60)]);
        s.add(9, 9);
        assert_eq!(s.len(), 1 + 35 + 10);
    }

    #[test]
    fn covering_needs_one_whole_range() {
        let s = RangeSet::from_items(&[(10, 20), (30, 40)]);
        assert!(s.covers(10, 20));
        assert!(s.covers(12, 15));
        assert!(!s.covers(15, 25));
        assert!(!s.covers(25, 35));
        assert!(!s.covers(0, 5));
        assert!(s.covers(7, 7));
    }

    #[test]
    fn gaps_are_found_in_order() {
        let total = 100;
        assert_eq!(RangeSet::new().first_gap(total), Some((0, 100)));
        assert_eq!(RangeSet::from_items(&[(0, 10)]).first_gap(total), Some((10, 100)));
        assert_eq!(RangeSet::from_items(&[(5, 10)]).first_gap(total), Some((0, 5)));
        assert_eq!(RangeSet::from_items(&[(0, 10), (20, 100)]).first_gap(total), Some((10, 20)));
        assert_eq!(RangeSet::from_items(&[(0, 100)]).first_gap(total), None);
        assert_eq!(RangeSet::from_items(&[(0, 200)]).first_gap(total), None);
    }
}
