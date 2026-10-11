// Capture the exact input forwarded to App::set_menus without changing TestPlatform.
use std::{cell::RefCell, rc::Rc};
use gpui::{Menu, MenuItem, OwnedMenu, OwnedMenuItem, OwnedOsMenu};

#[derive(Default)]
struct Publication { count: usize, menus: Vec<OwnedMenu> }

thread_local! {
    static OBSERVER: RefCell<Option<Rc<RefCell<Publication>>>> = const { RefCell::new(None) };
}

pub(crate) struct Guard {
    current: Rc<RefCell<Publication>>,
    previous: Option<Rc<RefCell<Publication>>>,
}

pub(crate) fn observe() -> Guard {
    let current = Rc::new(RefCell::new(Publication::default()));
    let previous = OBSERVER.with(|slot| slot.replace(Some(current.clone())));
    Guard { current, previous }
}

impl Guard {
    pub(crate) fn snapshot(&self) -> (usize, Vec<OwnedMenu>) {
        let current = self.current.borrow();
        (current.count, current.menus.clone())
    }
}

impl Drop for Guard {
    fn drop(&mut self) { OBSERVER.with(|slot| *slot.borrow_mut() = self.previous.take()); }
}

pub(super) fn capture(menus: &[Menu]) {
    OBSERVER.with(|slot| {
        if let Some(observer) = slot.borrow().as_ref() {
            let mut current = observer.borrow_mut();
            current.count = current.count.saturating_add(1);
            current.menus = menus.iter().map(owned_menu).collect();
        }
    });
}

fn owned_menu(menu: &Menu) -> OwnedMenu {
    OwnedMenu { name: menu.name.clone(), items: menu.items.iter().map(|item| match item {
        MenuItem::Separator => OwnedMenuItem::Separator,
        MenuItem::Submenu(menu) => OwnedMenuItem::Submenu(owned_menu(menu)),
        MenuItem::SystemMenu(menu) => OwnedMenuItem::SystemMenu(OwnedOsMenu {
            name: menu.name.clone(), menu_type: menu.menu_type,
        }),
        MenuItem::Action { name, action, os_action, checked } => OwnedMenuItem::Action {
            name: name.to_string(), action: action.boxed_clone(), os_action: *os_action, checked: *checked,
        },
    }).collect() }
}

#[test]
fn observer_keeps_menu_actions_checkmarks_and_system_items() {
    let observer = observe();
    capture(&[Menu { name: "test".into(), items: vec![
        MenuItem::Action { name: "copy".into(), action: Box::new(crate::actions::EditCopy),
            os_action: Some(gpui::OsAction::Copy), checked: true },
        MenuItem::Separator,
        MenuItem::Submenu(Menu { name: "child".into(), items: vec![] }),
        MenuItem::os_submenu("services", gpui::SystemMenuType::Services),
    ] }]);
    let (count, menus) = observer.snapshot();
    assert_eq!(count, 1); assert_eq!(menus[0].name.as_ref(), "test");
    match &menus[0].items[0] {
        OwnedMenuItem::Action { name, action, os_action, checked } => {
            assert_eq!(name, "copy"); assert!(action.as_any().is::<crate::actions::EditCopy>());
            assert!(*os_action == Some(gpui::OsAction::Copy)); assert!(*checked);
        }
        _ => panic!("expected captured action"),
    }
    assert!(matches!(&menus[0].items[1], OwnedMenuItem::Separator));
    assert!(matches!(&menus[0].items[2], OwnedMenuItem::Submenu(menu) if menu.name.as_ref() == "child"));
    assert!(matches!(&menus[0].items[3], OwnedMenuItem::SystemMenu(menu)
        if menu.name.as_ref() == "services" && menu.menu_type == gpui::SystemMenuType::Services));
}

#[test]
fn nested_observers_restore_the_previous_capture() {
    let outer = observe();
    capture(&[]);
    {
        let inner = observe();
        capture(&[]); capture(&[]);
        assert_eq!(inner.snapshot().0, 2); assert_eq!(outer.snapshot().0, 1);
    }
    capture(&[]);
    assert_eq!(outer.snapshot().0, 2);
}
