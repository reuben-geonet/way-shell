use std::cell::OnceCell;

thread_local! {
    static RESOURCE: OnceCell<gio::Resource> = const { OnceCell::new() };
}

/// The compiled resource is immutable and registered once on its owning thread.
pub fn get() -> gio::Resource {
    RESOURCE.with(|resource| {
        resource
            .get_or_init(|| {
                let bytes = glib::Bytes::from_static(include_bytes!(concat!(
                    env!("OUT_DIR"),
                    "/way-shell.gresource"
                )));
                let resource =
                    gio::Resource::from_data(&bytes).expect("compiled application resource");
                gio::resources_register(&resource);
                resource
            })
            .clone()
    })
}
