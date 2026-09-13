use std::cell::RefCell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use crate::util::format_size;

mod imp {
    use super::*;

    /// A file of a torrent, as the Files page lists it.
    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::FileItem)]
    pub struct FileItem {
        #[property(get, construct_only)]
        path: RefCell<String>,
        #[property(get, construct_only)]
        size_text: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FileItem {
        const NAME: &'static str = "RillFileItem";
        type Type = super::FileItem;
    }

    #[glib::derived_properties]
    impl ObjectImpl for FileItem {}
}

glib::wrapper! {
    pub struct FileItem(ObjectSubclass<imp::FileItem>);
}

impl FileItem {
    pub fn new(path: &str, size: u64) -> Self {
        glib::Object::builder()
            .property("path", path)
            .property("size-text", format_size(size))
            .build()
    }
}
