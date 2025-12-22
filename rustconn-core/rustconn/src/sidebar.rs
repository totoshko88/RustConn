//! Connection tree sidebar

use gtk4::prelude::*;
use gtk4::subclass::prelude::ObjectSubclassIsExt;
use gtk4::{
    gdk, gio, glib, Box as GtkBox, Button, DragSource, DropTarget, EventControllerKey,
    GestureClick, Label, ListItem, ListView, Orientation, PolicyType, ScrolledWindow,
    SearchEntry, SignalListItemFactory, SingleSelection, TreeExpander,
    TreeListModel, TreeListRow, Widget,
};

pub struct ConnectionSidebar {
    container: GtkBox,
    search_entry: SearchEntry,
    list_view: ListView,
    store: gio::ListStore,
}

impl ConnectionSidebar {
    #[must_use]
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_width_request(250);
        container.add_css_class("sidebar");

        let search_entry = SearchEntry::new();
        search_entry.set_placeholder_text(Some("Search connections..."));
        search_entry.set_margin_all(8);
        container.append(&search_entry);

        let store = gio::ListStore::new::<ConnectionItem>();
        let tree_model = TreeListModel::new(store.clone(), false, true, |item| {
            item.downcast_ref::<ConnectionItem>().and_then(|conn_item| conn_item.children())
        });

        let selection_model = SingleSelection::new(Some(tree_model));
        let factory = SignalListItemFactory::new();
        factory.connect_setup(|factory, obj| {
            if let Some(list_item) = obj.downcast_ref::<ListItem>() {
                Self::setup_list_item(factory, list_item);
            }
        });
        factory.connect_bind(|factory, obj| {
            if let Some(list_item) = obj.downcast_ref::<ListItem>() {
                Self::bind_list_item(factory, list_item);
            }
        });

        let list_view = ListView::new(Some(selection_model), Some(factory));
        list_view.add_css_class("navigation-sidebar");

        let key_controller = EventControllerKey::new();
        key_controller.connect_key_pressed(|_controller, key, _code, _state| {
            match key {
                gdk::Key::Return | gdk::Key::KP_Enter => glib::Propagation::Stop,
                _ => glib::Propagation::Proceed,
            }
        });
        list_view.add_controller(key_controller);

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Never)
            .vscrollbar_policy(PolicyType::Automatic)
            .vexpand(true)
            .child(&list_view)
            .build();
        container.append(&scrolled);

        let button_box = Self::create_button_box();
        container.append(&button_box);

        Self { container, search_entry, list_view, store }
    }

    fn create_button_box() -> GtkBox {
        let button_box = GtkBox::new(Orientation::Horizontal, 4);
        button_box.set_margin_all(8);
        button_box.set_halign(gtk4::Align::Center);

        let add_button = Button::from_icon_name("list-add-symbolic");
        add_button.set_tooltip_text(Some("Add Connection"));
        button_box.append(&add_button);

        let add_group_button = Button::from_icon_name("folder-new-symbolic");
        add_group_button.set_tooltip_text(Some("Add Group"));
        button_box.append(&add_group_button);

        let import_button = Button::from_icon_name("document-open-symbolic");
        import_button.set_tooltip_text(Some("Import Connections"));
        button_box.append(&import_button);

        button_box
    }

    fn setup_list_item(_factory: &SignalListItemFactory, list_item: &ListItem) {
        let expander = TreeExpander::new();
        let content_box = GtkBox::new(Orientation::Horizontal, 8);
        content_box.set_margin_all(4);

        let icon = gtk4::Image::from_icon_name("network-server-symbolic");
        content_box.append(&icon);

        let label = Label::new(None);
        label.set_halign(gtk4::Align::Start);
        label.set_hexpand(true);
        content_box.append(&label);

        expander.set_child(Some(&content_box));
        list_item.set_child(Some(&expander));

        let drag_source = DragSource::new();
        drag_source.set_actions(gdk::DragAction::MOVE);
        drag_source.connect_prepare(|_source, _x, _y| {
            Some(gdk::ContentProvider::for_value(&"connection".to_value()))
        });
        expander.add_controller(drag_source);

        let drop_target = DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
        drop_target.connect_drop(|_target, _value, _x, _y| true);
        expander.add_controller(drop_target);

        let gesture = GestureClick::new();
        gesture.set_button(gdk::BUTTON_SECONDARY);
        gesture.connect_pressed(|gesture, _n_press, x, y| {
            if let Some(widget) = gesture.widget() {
                Self::show_context_menu(&widget, x, y);
            }
        });
        expander.add_controller(gesture);
    }

    fn bind_list_item(_factory: &SignalListItemFactory, list_item: &ListItem) {
        let Some(expander) = list_item.child().and_downcast::<TreeExpander>() else { return; };
        let Some(row) = list_item.item().and_downcast::<TreeListRow>() else { return; };
        expander.set_list_row(Some(&row));
        let Some(item) = row.item().and_downcast::<ConnectionItem>() else { return; };
        let Some(content_box) = expander.child().and_downcast::<GtkBox>() else { return; };

        if let Some(icon) = content_box.first_child().and_downcast::<gtk4::Image>() {
            if item.is_group() {
                icon.set_icon_name(Some("folder-symbolic"));
            } else {
                let icon_name = match item.protocol().as_str() {
                    "ssh" => "utilities-terminal-symbolic",
                    "rdp" => "computer-symbolic",
                    "vnc" => "video-display-symbolic",
                    _ => "network-server-symbolic",
                };
                icon.set_icon_name(Some(icon_name));
            }
        }

        if let Some(label) = content_box.last_child().and_downcast::<Label>() {
            label.set_text(&item.name());
        }
    }

    fn show_context_menu(widget: &impl IsA<Widget>, x: f64, y: f64) {
        let menu = gio::Menu::new();
        menu.append(Some("Connect"), Some("win.connect"));
        menu.append(Some("Edit"), Some("win.edit-connection"));
        menu.append(Some("Duplicate"), Some("win.duplicate-connection"));
        menu.append(Some("Delete"), Some("win.delete-connection"));

        let popover = gtk4::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(widget);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.set_autohide(true);

        // Connect to closed signal to unparent the popover before destruction
        // This prevents GTK warning: "Broken accounting of active state"
        popover.connect_closed(|p| {
            p.unparent();
        });

        popover.popup();
    }

    #[must_use] pub fn widget(&self) -> &GtkBox { &self.container }
    #[must_use] pub fn search_entry(&self) -> &SearchEntry { &self.search_entry }
    #[must_use] pub fn list_view(&self) -> &ListView { &self.list_view }
    #[must_use] pub fn store(&self) -> &gio::ListStore { &self.store }
}

impl Default for ConnectionSidebar {
    fn default() -> Self { Self::new() }
}

mod imp {
    use super::*;
    use glib::subclass::prelude::*;
    use glib::Properties;
    use std::cell::RefCell;

    #[derive(Default, Properties)]
    #[properties(wrapper_type = super::ConnectionItem)]
    pub struct ConnectionItem {
        #[property(get, set)] id: RefCell<String>,
        #[property(get, set)] name: RefCell<String>,
        #[property(get, set)] protocol: RefCell<String>,
        #[property(get, set)] is_group: RefCell<bool>,
        #[property(get, set)] host: RefCell<String>,
        pub(super) children: RefCell<Option<gio::ListStore>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ConnectionItem {
        const NAME: &'static str = "RustConnConnectionItem";
        type Type = super::ConnectionItem;
    }

    #[glib::derived_properties]
    impl ObjectImpl for ConnectionItem {}
}

glib::wrapper! {
    pub struct ConnectionItem(ObjectSubclass<imp::ConnectionItem>);
}

impl ConnectionItem {
    #[must_use]
    pub fn new_connection(id: &str, name: &str, protocol: &str, host: &str) -> Self {
        glib::Object::builder()
            .property("id", id)
            .property("name", name)
            .property("protocol", protocol)
            .property("is-group", false)
            .property("host", host)
            .build()
    }

    #[must_use]
    pub fn new_group(id: &str, name: &str) -> Self {
        let item: Self = glib::Object::builder()
            .property("id", id)
            .property("name", name)
            .property("protocol", "")
            .property("is-group", true)
            .property("host", "")
            .build();
        *item.imp().children.borrow_mut() = Some(gio::ListStore::new::<ConnectionItem>());
        item
    }

    pub fn children(&self) -> Option<gio::ListModel> {
        self.imp().children.borrow().as_ref().map(|store| store.clone().upcast())
    }

    pub fn add_child(&self, child: &ConnectionItem) {
        if let Some(ref store) = *self.imp().children.borrow() {
            store.append(child);
        }
    }
}

impl Default for ConnectionItem {
    fn default() -> Self { Self::new_connection("", "Unnamed", "ssh", "") }
}
