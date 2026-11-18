//! Actions a pane takes from wherever the focus is in the window.
//!
//! An action runs where the focus is: GPUI hands it to the focused element and then to
//! each element around it, out to the Frame's root. A pane that has no focus is not on
//! that way, so a key bound for the whole window, as the one that opens a terminal, would
//! reach no pane but the one with the focus. A pane therefore names here, when it is
//! built, the actions it takes from anywhere; the Frame's root element hands each of them
//! to that pane, unless an element nearer the focus took it first.
//!
//! This is the one way an action reaches a pane that does not hold the focus. The Frame
//! does not know the panes or their actions; it holds what the panes put here.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::*;

/// One route: adds a listener for one action to the Frame's root element.
type Route = Rc<dyn Fn(Div) -> Div>;

/// The routes of one window. Cheap to clone; every clone holds the same routes.
#[derive(Clone, Default)]
pub struct Routes(Rc<RefCell<Vec<Route>>>);

impl Routes {
    /// Hands the action `A` to `pane`, through `take`, whenever it is run in the window
    /// and no element on the way from the focus to the Frame's root takes it first.
    ///
    /// A pane calls this while it is built, once for each action it takes from anywhere.
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

    /// The Frame's root element with a listener for every routed action.
    pub(crate) fn apply(&self, root: Div) -> Div {
        self.0.borrow().iter().fold(root, |root, route| route(root))
    }
}
