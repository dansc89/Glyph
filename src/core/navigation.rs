//! Bounded browser-style navigation history for complete locations.
#[derive(Debug)]
pub struct NavigationHistory<T: Copy + PartialEq = usize> {
    back: std::collections::VecDeque<T>,
    forward: Vec<T>,
}

// Deriving Default would unnecessarily require T: Default.
impl<T: Copy + PartialEq> Default for NavigationHistory<T> {
    fn default() -> Self {
        Self {
            back: std::collections::VecDeque::new(),
            forward: Vec::new(),
        }
    }
}

impl<T: Copy + PartialEq> NavigationHistory<T> {
    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }
    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }
    pub fn visit(&mut self, current: T, next: T) {
        if current == next {
            return;
        }
        self.push_back(current);
        self.forward.clear();
    }
    fn push_back(&mut self, page: T) {
        if self.back.len() == 256 {
            self.back.pop_front();
        }
        self.back.push_back(page);
    }
    pub fn back(&mut self, current: T) -> Option<T> {
        let page = self.back.pop_back()?;
        self.forward.push(current);
        Some(page)
    }
    pub fn forward(&mut self, current: T) -> Option<T> {
        let page = self.forward.pop()?;
        self.push_back(current);
        Some(page)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Copy, Clone, PartialEq)]
    struct Viewport {
        page: usize,
        zoom: f32,
        pan: (f32, f32),
    }

    #[test]
    fn back_and_forward_restore_the_complete_viewport() {
        // Deliberately no Default implementation: history stores values, not defaults.
        let first = Viewport {
            page: 2,
            zoom: 1.5,
            pan: (12.0, -30.0),
        };
        let second = Viewport {
            page: 7,
            zoom: 2.25,
            pan: (-80.0, 45.0),
        };
        let mut history = NavigationHistory::<Viewport>::default();
        assert!(!history.can_back());
        assert!(!history.can_forward());
        history.visit(first, second);
        assert!(history.can_back());
        assert_eq!(history.back(second), Some(first));
        assert!(history.can_forward());
        assert_eq!(history.forward(first), Some(second));
        assert_eq!(history.forward(second), None);
        assert_eq!(history.back(second), Some(first));
        assert_eq!(history.back(first), None);
    }

    #[test]
    fn back_and_forward_restore_pages_without_recording_duplicates() {
        let mut history = NavigationHistory::default();
        history.visit(0, 2);
        history.visit(2, 2);
        history.visit(2, 7);
        assert_eq!(history.back(7), Some(2));
        assert_eq!(history.back(2), Some(0));
        assert_eq!(history.back(0), None);
        assert_eq!(history.forward(0), Some(2));
        assert_eq!(history.forward(2), Some(7));
    }
    #[test]
    fn visiting_a_new_page_discards_forward_history() {
        let mut history = NavigationHistory::default();
        history.visit(0, 2);
        assert_eq!(history.back(2), Some(0));
        history.visit(0, 4);
        assert_eq!(history.forward(4), None);
        assert_eq!(history.back(4), Some(0));
    }
    #[test]
    fn same_page_view_changes_do_not_record_page_navigation() {
        let first = Viewport {
            page: 2,
            zoom: 1.0,
            pan: (0.0, 0.0),
        };
        let second = Viewport {
            page: 7,
            zoom: 2.0,
            pan: (10.0, 20.0),
        };
        let adjusted = Viewport {
            zoom: 3.0,
            pan: (-40.0, 60.0),
            ..second
        };
        let mut history = NavigationHistory::<Viewport>::default();
        history.visit(first, second);
        // The caller records only page navigation; history compares whole values.
        if second.page != adjusted.page {
            history.visit(second, adjusted);
        }
        assert_eq!(history.back(adjusted), Some(first));
        assert_eq!(history.back(first), None);
        assert_eq!(history.forward(first), Some(adjusted));
        assert_eq!(history.forward(adjusted), None);
    }

    #[test]
    fn branching_after_multiple_back_steps_discards_the_entire_forward_stack() {
        let mut history = NavigationHistory::default();
        history.visit(0, 1);
        history.visit(1, 2);
        history.visit(2, 3);
        assert_eq!(history.back(3), Some(2));
        assert_eq!(history.back(2), Some(1));
        history.visit(1, 1);
        assert!(history.can_forward());
        history.visit(1, 9);
        assert!(!history.can_forward());
        assert_eq!(history.forward(9), None);
        assert_eq!(history.back(9), Some(1));
        assert_eq!(history.back(1), Some(0));
        assert_eq!(history.back(0), None);
        assert_eq!(history.forward(0), Some(1));
        assert_eq!(history.forward(1), Some(9));
        assert_eq!(history.forward(9), None);
    }

    #[test]
    fn combined_stacks_stay_bounded_and_retain_the_newest_locations() {
        let mut history = NavigationHistory::default();
        for page in 0..1000 {
            history.visit(page, page + 1);
            assert!(history.back.len() + history.forward.len() <= 256);
        }
        let mut current = 1000;
        for expected in (744..1000).rev() {
            current = history.back(current).expect("retained back location");
            assert_eq!(current, expected);
            assert_eq!(history.back.len() + history.forward.len(), 256);
        }
        assert_eq!(history.back(current), None);
        assert_eq!(history.forward.len(), 256);
        for expected in 745..=1000 {
            current = history.forward(current).expect("retained forward location");
            assert_eq!(current, expected);
            assert_eq!(history.back.len() + history.forward.len(), 256);
        }
        assert_eq!(history.forward(current), None);
        // Branch with both stacks populated after reaching capacity.
        for _ in 0..128 {
            current = history.back(current).unwrap();
        }
        history.visit(current, 2000);
        assert!(!history.can_forward());
        assert_eq!(history.back.len() + history.forward.len(), 129);
        assert_eq!(history.back(2000), Some(current));
        assert_eq!(history.forward(current), Some(2000));
    }

    #[test]
    fn history_is_bounded() {
        let mut history = NavigationHistory::default();
        for page in 0..1000 {
            history.visit(page, page + 1);
        }
        assert_eq!(history.back.len(), 256);
    }
}
