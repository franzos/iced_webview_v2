use std::collections::HashMap;

use super::ViewId;

pub struct ViewManager<V> {
    views: HashMap<ViewId, V>,
}

impl<V> Default for ViewManager<V> {
    fn default() -> Self {
        Self {
            views: HashMap::new(),
        }
    }
}

impl<V> ViewManager<V> {
    pub fn get(&self, id: ViewId) -> Option<&V> {
        self.views.get(&id)
    }

    pub fn get_mut(&mut self, id: ViewId) -> Option<&mut V> {
        self.views.get_mut(&id)
    }

    pub fn insert(&mut self, view: V) -> ViewId {
        let id = rand::random::<ViewId>();
        self.views.insert(id, view);
        id
    }

    pub fn remove(&mut self, id: ViewId) -> Option<V> {
        self.views.remove(&id)
    }

    pub fn contains(&self, id: ViewId) -> bool {
        self.views.contains_key(&id)
    }

    pub fn keys(&self) -> Vec<ViewId> {
        self.views.keys().copied().collect()
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.views.values()
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.views.values_mut()
    }

    pub fn iter(&self) -> impl Iterator<Item = (ViewId, &V)> {
        self.views.iter().map(|(&id, v)| (id, v))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (ViewId, &mut V)> {
        self.views.iter_mut().map(|(&id, v)| (id, v))
    }

    pub fn len(&self) -> usize {
        self.views.len()
    }

    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }
}
