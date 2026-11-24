//! Actions a pane takes from wherever the focus is: GPUI hands an action to the focused element,
//! then to the Frame's root, so a pane without focus would miss a window-wide key.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::*;

type Route = Rc<dyn Fn(Div) -> Div>;

#[derive(Clone, Default)]
pub struct Routes(Rc<RefCell<Vec<Route>>>);

impl Routes {
    /// The route holds the pane weakly: a pane that is gone takes nothing.
    pub fn route<A: Action, V: 'static>(
        &self,
        pane: &Entity<V>,
        take: fn(&mut V, &A, &mut Window, &mut Context<V>),
    ) {
        let pane = pane.downgrade();
        let route: Route = Rc::new(move |root: Div| {
            let pane = pane.clone();
            root.on_action(move |action: &A, window: &mut Window, cx: &mut App| {
                pane.update(cx, |pane, cx| take(pane, action, window, cx))
                    .ok();
            })
        });
        self.0.borrow_mut().push(route);
    }

    pub(crate) fn apply(&self, root: Div) -> Div {
        self.0.borrow().iter().fold(root, |root, route| route(root))
    }
}
