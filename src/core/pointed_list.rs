//! A circular list with a cursor ("point").
//!
//! Port of the Python `lib/pointed_list.py`. The Python version asserts on an
//! empty list and on out-of-range `goto`; here those become typed results
//! (`Option` on construction, `bool` on `goto`) so misuse can't panic silently.

/// A non-empty list plus an index into it. `next`/`prev` wrap around.
///
/// The non-empty invariant is guaranteed at construction. `delete` can still
/// empty the list (mirroring the Python behaviour); once empty, `current`
/// panics — callers must stop navigating, exactly as the old code assumed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointedList<T> {
    list: Vec<T>,
    index: usize,
}

impl<T: Clone + PartialEq> PointedList<T> {
    /// Build from a list. Returns `None` if empty (the Python `assert`).
    pub fn new(list: Vec<T>) -> Option<Self> {
        if list.is_empty() {
            None
        } else {
            Some(Self { list, index: 0 })
        }
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// The item under the cursor. Panics if the list has been emptied by
    /// `delete` (matches Python's `IndexError`).
    pub fn current(&self) -> &T {
        &self.list[self.index]
    }

    /// Advance the cursor (wrapping) and return the new current item.
    pub fn next(&mut self) -> &T {
        self.index = (self.index + 1) % self.list.len();
        self.current()
    }

    /// Retreat the cursor (wrapping) and return the new current item.
    pub fn prev(&mut self) -> &T {
        // rem_euclid keeps the result non-negative when index is 0.
        self.index = (self.index + self.list.len() - 1) % self.list.len();
        self.current()
    }

    /// Jump to `index`. Returns `false` (and does nothing) if out of range,
    /// where the Python code would assert.
    pub fn goto(&mut self, index: usize) -> bool {
        if index < self.list.len() {
            self.index = index;
            true
        } else {
            false
        }
    }

    /// Jump to the first item equal to `value`. No-op if not present.
    pub fn goto_value(&mut self, value: &T) {
        if let Some(i) = self.list.iter().position(|x| x == value) {
            self.index = i;
        }
    }

    /// Remove the current item. The cursor wraps to stay in range and the new
    /// current item is returned (`None` if the list is now empty).
    pub fn delete(&mut self) -> Option<T> {
        self.list.remove(self.index);
        if self.list.is_empty() {
            self.index = 0;
            None
        } else {
            self.index %= self.list.len();
            Some(self.current().clone())
        }
    }

    pub fn contains(&self, value: &T) -> bool {
        self.list.contains(value)
    }

    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.list.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Construction ---

    #[test]
    fn starts_at_index_zero() {
        let pl = PointedList::new(vec![1, 2, 3]).unwrap();
        assert_eq!(pl.index(), 0);
        assert_eq!(*pl.current(), 1);
    }

    #[test]
    fn empty_list_is_none() {
        assert!(PointedList::<i32>::new(vec![]).is_none());
    }

    // --- Navigation ---

    #[test]
    fn next_moves_forward() {
        let mut pl = PointedList::new(vec!["a", "b", "c"]).unwrap();
        assert_eq!(*pl.next(), "b");
        assert_eq!(*pl.next(), "c");
    }

    #[test]
    fn next_wraps_around() {
        let mut pl = PointedList::new(vec![1, 2, 3]).unwrap();
        pl.next();
        pl.next();
        assert_eq!(*pl.next(), 1);
    }

    #[test]
    fn prev_moves_backward() {
        let mut pl = PointedList::new(vec!["a", "b", "c"]).unwrap();
        pl.goto(2);
        assert_eq!(*pl.prev(), "b");
        assert_eq!(*pl.prev(), "a");
    }

    #[test]
    fn prev_wraps_around() {
        let mut pl = PointedList::new(vec![1, 2, 3]).unwrap();
        assert_eq!(*pl.prev(), 3);
    }

    #[test]
    fn single_element_next_stays() {
        let mut pl = PointedList::new(vec!["only"]).unwrap();
        assert_eq!(*pl.next(), "only");
        assert_eq!(pl.index(), 0);
    }

    #[test]
    fn single_element_prev_stays() {
        let mut pl = PointedList::new(vec!["only"]).unwrap();
        assert_eq!(*pl.prev(), "only");
        assert_eq!(pl.index(), 0);
    }

    // --- Goto ---

    #[test]
    fn goto_valid_index() {
        let mut pl = PointedList::new(vec![10, 20, 30, 40]).unwrap();
        assert!(pl.goto(2));
        assert_eq!(*pl.current(), 30);
    }

    #[test]
    fn goto_first_element() {
        let mut pl = PointedList::new(vec![1, 2, 3]).unwrap();
        pl.goto(2);
        pl.goto(0);
        assert_eq!(*pl.current(), 1);
    }

    #[test]
    fn goto_last_element() {
        let mut pl = PointedList::new(vec![1, 2, 3]).unwrap();
        pl.goto(2);
        assert_eq!(*pl.current(), 3);
    }

    #[test]
    fn goto_out_of_bounds_rejected() {
        let mut pl = PointedList::new(vec![1, 2, 3]).unwrap();
        assert!(!pl.goto(3));
        assert_eq!(pl.index(), 0); // unchanged
    }

    // --- GotoValue ---

    #[test]
    fn goto_value_existing() {
        let mut pl = PointedList::new(vec!["apple", "banana", "cherry"]).unwrap();
        pl.goto_value(&"banana");
        assert_eq!(*pl.current(), "banana");
        assert_eq!(pl.index(), 1);
    }

    #[test]
    fn goto_value_nonexisting_does_nothing() {
        let mut pl = PointedList::new(vec!["a", "b", "c"]).unwrap();
        pl.goto(1);
        pl.goto_value(&"not_here");
        assert_eq!(pl.index(), 1);
    }

    #[test]
    fn goto_value_first_occurrence() {
        let mut pl = PointedList::new(vec![1, 2, 1, 3]).unwrap();
        pl.goto_value(&1);
        assert_eq!(pl.index(), 0);
    }

    // --- Delete ---

    #[test]
    fn delete_middle_element() {
        let mut pl = PointedList::new(vec![1, 2, 3]).unwrap();
        pl.goto(1);
        let result = pl.delete();
        assert_eq!(pl.iter().copied().collect::<Vec<_>>(), vec![1, 3]);
        assert_eq!(result, Some(3));
        assert_eq!(pl.index(), 1);
    }

    #[test]
    fn delete_first_element() {
        let mut pl = PointedList::new(vec![1, 2, 3]).unwrap();
        let result = pl.delete();
        assert_eq!(pl.iter().copied().collect::<Vec<_>>(), vec![2, 3]);
        assert_eq!(result, Some(2));
        assert_eq!(pl.index(), 0);
    }

    #[test]
    fn delete_last_element_wraps_index() {
        let mut pl = PointedList::new(vec![1, 2, 3]).unwrap();
        pl.goto(2);
        let result = pl.delete();
        assert_eq!(pl.iter().copied().collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(pl.index(), 0);
        assert_eq!(result, Some(1));
    }

    #[test]
    fn delete_until_one_element() {
        let mut pl = PointedList::new(vec![1, 2]).unwrap();
        pl.delete();
        assert_eq!(pl.iter().copied().collect::<Vec<_>>(), vec![2]);
        assert_eq!(pl.index(), 0);
    }

    #[test]
    fn delete_last_remaining_empties_list() {
        let mut pl = PointedList::new(vec![1]).unwrap();
        assert_eq!(pl.delete(), None);
        assert!(pl.is_empty());
    }

    // --- Iteration ---

    #[test]
    fn iterates_all_elements() {
        let pl = PointedList::new(vec![1, 2, 3]).unwrap();
        assert_eq!(pl.iter().copied().collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    #[test]
    fn iteration_does_not_affect_index() {
        let mut pl = PointedList::new(vec![1, 2, 3]).unwrap();
        pl.goto(1);
        let _ = pl.iter().count();
        assert_eq!(pl.index(), 1);
    }
}
