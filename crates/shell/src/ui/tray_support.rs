//! Shared native presentation helpers for tray items and menus.
use crate::services::tray::{RgbaImage, menu::MenuNode};
use gio::prelude::*;
use gtk::gdk_pixbuf::{Colorspace, Pixbuf};

pub fn rgba_pixbuf(image: &RgbaImage) -> Option<Pixbuf> {
    let width = i32::try_from(image.width).ok().filter(|width| *width > 0)?;
    let height = i32::try_from(image.height)
        .ok()
        .filter(|height| *height > 0)?;
    let stride = width.checked_mul(4)?;
    let length = (stride as usize).checked_mul(height as usize)?;
    if image.pixels.len() != length {
        return None;
    }
    // GBytes retains these bytes even after the tray record or source variant
    // disappears, including when a GTK texture still retains the pixbuf.
    Some(Pixbuf::from_bytes(
        &glib::Bytes::from_owned(image.pixels.clone()),
        Colorspace::Rgb,
        true,
        8,
        width,
        height,
        stride,
    ))
}

pub fn menu_model(root: &MenuNode, key: &str) -> gio::Menu {
    let menu = gio::Menu::new();
    let mut section: Option<gio::Menu> = None;
    for child in root.children.iter().filter(|child| child.visible) {
        if child.separator {
            if let Some(previous) = section.replace(gio::Menu::new()) {
                menu.append_section(None, &previous);
            }
            continue;
        }
        let item = gio::MenuItem::new(Some(&child.label), None);
        item.set_action_and_target_value(
            Some("sni.item-clicked"),
            Some(&(key, child.id).to_variant()),
        );
        if !child.children.is_empty() {
            item.set_submenu(Some(&menu_model(child, key)));
        }
        section.as_ref().unwrap_or(&menu).append_item(&item);
    }
    if let Some(section) = section {
        menu.append_section(None, &section);
    }
    menu
}

pub async fn load_theme_icon(directory: &str, name: &str) -> Result<Pixbuf, glib::Error> {
    let directory = gio::File::for_path(directory);
    let file = if name.contains('.') {
        directory.child(name)
    } else {
        let entries = directory
            .enumerate_children_future(
                "standard::name",
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
            )
            .await?;
        let result = async {
            loop {
                let batch = entries
                    .next_files_future(64, glib::Priority::DEFAULT)
                    .await?;
                if batch.is_empty() {
                    break;
                }
                for entry in batch {
                    let path = entry.name();
                    let text = path.to_string_lossy();
                    if text.starts_with(name)
                        && [".png", ".svg", ".xpm", ".ico"]
                            .iter()
                            .any(|suffix| text.ends_with(suffix))
                    {
                        return Ok(directory.child(path));
                    }
                }
            }
            Err(glib::Error::new(
                gio::IOErrorEnum::NotFound,
                "icon was absent from its theme directory",
            ))
        }
        .await;
        let _ = entries.close_future(glib::Priority::DEFAULT).await;
        result?
    };
    let info = file
        .query_info_future(
            "standard::type",
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await?;
    if info.file_type() != gio::FileType::Regular {
        return Err(glib::Error::new(
            gio::IOErrorEnum::InvalidArgument,
            "tray icon is not a regular file",
        ));
    }
    let stream = file.read_future(glib::Priority::DEFAULT).await?;
    let result = Pixbuf::from_stream_at_scale_future(&stream, 256, 256, true).await;
    let _ = stream.close_future(glib::Priority::DEFAULT).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_icons_decode_png_and_reject_missing_or_non_file_paths() {
        let directory =
            std::env::temp_dir().join(format!("way-shell-tray-decode-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(
            directory.join("fixture.png"),
            include_bytes!("../../../../tests/fixtures/tray-icon.png"),
        )
        .unwrap();
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                context.block_on(async {
                    for name in ["fixture", "fixture.png"] {
                        let image = load_theme_icon(directory.to_str().unwrap(), name)
                            .await
                            .unwrap();
                        assert_eq!((image.width(), image.height()), (256, 256));
                        assert_eq!(&image.read_pixel_bytes().as_ref()[..4], &[70, 80, 90, 255]);
                    }
                    assert!(
                        load_theme_icon(directory.to_str().unwrap(), "missing")
                            .await
                            .is_err()
                    );
                    std::fs::create_dir(directory.join("not-a-file.png")).unwrap();
                    assert!(
                        load_theme_icon(directory.to_str().unwrap(), "not-a-file.png")
                            .await
                            .is_err()
                    );
                })
            })
            .unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rgba_images_own_their_bytes_and_reject_invalid_native_dimensions() {
        let source = RgbaImage {
            width: 2,
            height: 1,
            pixels: vec![11, 22, 33, 128, 44, 55, 66, 64],
        };
        let image = rgba_pixbuf(&source).unwrap();
        drop(source);
        assert_eq!(
            image.read_pixel_bytes().as_ref(),
            &[11, 22, 33, 128, 44, 55, 66, 64]
        );
        for source in [
            RgbaImage {
                width: 0,
                height: 1,
                pixels: vec![],
            },
            RgbaImage {
                width: i32::MAX as u32,
                height: 2,
                pixels: vec![],
            },
            RgbaImage {
                width: 1_073_741_825,
                height: 4,
                pixels: vec![0; 16],
            },
            RgbaImage {
                width: 2,
                height: 1,
                pixels: vec![0; 4],
            },
        ] {
            assert!(rgba_pixbuf(&source).is_none());
        }
    }

    #[test]
    fn menu_sections_keep_hidden_submenus_and_stable_signed_targets() {
        let leaf = |id, label: &str| MenuNode {
            id,
            label: label.into(),
            visible: true,
            separator: false,
            children: vec![],
        };
        let mut hidden = leaf(2, "Hidden");
        hidden.visible = false;
        let mut separator = leaf(3, "");
        separator.separator = true;
        let mut submenu = leaf(4, "Submenu");
        submenu.children = vec![hidden];
        let root = MenuNode {
            children: vec![
                leaf(i32::MAX, "_Open"),
                separator.clone(),
                separator,
                submenu,
            ],
            ..leaf(0, "")
        };
        let model = menu_model(&root, ":1.42/Tray");
        assert_eq!(model.n_items(), 3);
        assert_eq!(
            model
                .item_attribute_value(0, "label", None)
                .unwrap()
                .get::<String>()
                .as_deref(),
            Some("_Open")
        );
        assert_eq!(
            model
                .item_attribute_value(0, "target", None)
                .unwrap()
                .get::<(String, i32)>(),
            Some((":1.42/Tray".into(), i32::MAX))
        );
        assert_eq!(model.item_link(1, "section").unwrap().n_items(), 0);
        let section = model.item_link(2, "section").unwrap();
        assert_eq!(section.n_items(), 1);
        assert_eq!(section.item_link(0, "submenu").unwrap().n_items(), 0);
    }
}
