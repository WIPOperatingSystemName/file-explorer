use telorgon::app::*;

/// Preserve colored folder and file artwork; action glyphs follow the theme.
pub(super) fn icon_view(name: &str, size: f32) -> Image {
    use crate::assets::icons::*;
    let asset = match name {
        "home" => HOME,
        "folder" => FOLDER,
        "file" => FILE,
        "document" => DOCUMENT,
        "pdf" => PDF,
        "archive" => ARCHIVE,
        "image" | "pictures" => IMAGE,
        "audio" | "music" => AUDIO,
        "video" | "videos" => VIDEO,
        "code" => CODE,
        "desktop" => DESKTOP,
        "download" | "downloads" => DOWNLOAD,
        "drive" | "computer" => DRIVE,
        "pin" => PIN,
        "recycle" => RECYCLE,
        "back" => BACK,
        "forward" => FORWARD,
        "up" => UP,
        "refresh" => REFRESH,
        "plus" | "new" => PLUS,
        "close" => CLOSE,
        "cut" => CUT,
        "copy" => COPY,
        "paste" => PASTE,
        "rename" => RENAME,
        "trash" | "delete" => TRASH,
        "sort" => SORT,
        "view" | "grid" => GRID,
        "list" => LIST,
        "more" => MORE,
        "details" => DETAILS,
        "search" => SEARCH,
        "breadcrumb" | "chevron-right" => CHEVRON_RIGHT,
        "chevron-down" => CHEVRON_DOWN,
        "edit" => EDIT,
        "help" => HELP,
        "bookmark" => BOOKMARK,
        "check" => CHECK,
        _ => FILE,
    };
    let icon = image(asset).width(size).height(size);
    if matches!(
        name,
        "back"
            | "forward"
            | "up"
            | "refresh"
            | "plus"
            | "new"
            | "close"
            | "cut"
            | "copy"
            | "paste"
            | "rename"
            | "trash"
            | "delete"
            | "sort"
            | "view"
            | "grid"
            | "list"
            | "more"
            | "details"
            | "search"
            | "breadcrumb"
            | "chevron-right"
            | "chevron-down"
            | "edit"
            | "help"
            | "bookmark"
            | "check"
    ) {
        icon.tint(super::theme::TEXT)
    } else {
        icon
    }
}
