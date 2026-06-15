//! UI message catalogue and translations for demiurg.
//!
//! Every user-facing string is a [`Msg`] variant; [`tr`] resolves it for
//! a [`Lang`]. The per-language `match` arms are exhaustive, so adding a
//! `Msg` without translating it everywhere is a compile error — no
//! missing-key surprises for the artists relying on the localised UI.
//!
//! Dynamic parts (voxel counts, dimensions) stay out of the catalogue:
//! callers format `"{}: {n}"` with the translated label, so number/order
//! formatting is the caller's job, not the translator's.
//!
//! The crate is UI-framework-agnostic and dependency-free, so the native
//! editor and the future web build share one catalogue.

// `no_std` for real builds (wasm-friendly, zero deps); tests link std for
// the harness.
#![cfg_attr(not(test), no_std)]

/// A supported interface language.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Lang {
    #[default]
    En,
    Ru,
}

impl Lang {
    /// All languages, in menu order.
    #[must_use]
    pub const fn all() -> [Lang; 2] {
        [Lang::En, Lang::Ru]
    }

    /// The language's own name, for the language picker.
    #[must_use]
    pub const fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Ru => "Русский",
        }
    }

    /// Short code (`"en"`, `"ru"`), e.g. for a `DEMIURG_LANG` env var.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ru => "ru",
        }
    }

    /// Parse a short code (case-insensitive); `None` if unknown.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Lang> {
        let code = code.trim();
        if code.eq_ignore_ascii_case("en") {
            Some(Lang::En)
        } else if code.eq_ignore_ascii_case("ru") {
            Some(Lang::Ru)
        } else {
            None
        }
    }
}

/// A translatable UI string.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Msg {
    // Tool panel
    Tools,
    Place,
    Erase,
    Paint,
    Eyedropper,
    BoxTool,
    Sphere,
    FloodFill,
    Select,
    Radius,
    Colour,
    ModelColours,
    Mirror,
    Pivot,
    CenterPivot,
    Size,
    Crop,
    Resize,
    Grow,
    Voxels,
    // Reference image
    Reference,
    Side,
    Depth,
    Flip,
    Move,
    Show,
    Remove,
    // Selection
    Selected,
    Delete,
    Copy,
    Paste,
    // Camera view presets
    Views,
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
    // Menus
    File,
    New,
    OpenKv6,
    OpenVox,
    OpenProject,
    OpenReference,
    Save,
    SaveAs,
    ExportKv6,
    ExportVxl,
    ExportVox,
    Edit,
    Undo,
    Redo,
    View,
    Lighting,
    Grid,
    VoxelEdges,
    Render,
    RenderSprite,
    RenderVoxel,
    Language,
    // Help line
    HelpApply,
    HelpOrbit,
    HelpSelect,
    // Window title + quit confirmation
    Untitled,
    ConfirmQuitTitle,
    ConfirmQuitBody,
    QuitAnyway,
    Cancel,
    // Save progress + autosave recovery
    Saving,
    RecoveredTitle,
    RecoveredBody,
    Ok,
}

impl Msg {
    /// Every message, for catalogue-completeness tests / tooling.
    pub const ALL: [Msg; 72] = [
        Msg::Tools,
        Msg::Place,
        Msg::Erase,
        Msg::Paint,
        Msg::Eyedropper,
        Msg::BoxTool,
        Msg::Sphere,
        Msg::FloodFill,
        Msg::Select,
        Msg::Radius,
        Msg::Colour,
        Msg::ModelColours,
        Msg::Mirror,
        Msg::Pivot,
        Msg::CenterPivot,
        Msg::Size,
        Msg::Crop,
        Msg::Resize,
        Msg::Grow,
        Msg::Voxels,
        Msg::Reference,
        Msg::Side,
        Msg::Depth,
        Msg::Flip,
        Msg::Move,
        Msg::Show,
        Msg::Remove,
        Msg::Selected,
        Msg::Delete,
        Msg::Copy,
        Msg::Paste,
        Msg::Views,
        Msg::Front,
        Msg::Back,
        Msg::Left,
        Msg::Right,
        Msg::Top,
        Msg::Bottom,
        Msg::File,
        Msg::New,
        Msg::OpenKv6,
        Msg::OpenVox,
        Msg::OpenProject,
        Msg::OpenReference,
        Msg::Save,
        Msg::SaveAs,
        Msg::ExportKv6,
        Msg::ExportVxl,
        Msg::ExportVox,
        Msg::Edit,
        Msg::Undo,
        Msg::Redo,
        Msg::View,
        Msg::Lighting,
        Msg::Grid,
        Msg::VoxelEdges,
        Msg::Render,
        Msg::RenderSprite,
        Msg::RenderVoxel,
        Msg::Language,
        Msg::HelpApply,
        Msg::HelpOrbit,
        Msg::HelpSelect,
        Msg::Untitled,
        Msg::ConfirmQuitTitle,
        Msg::ConfirmQuitBody,
        Msg::QuitAnyway,
        Msg::Cancel,
        Msg::Saving,
        Msg::RecoveredTitle,
        Msg::RecoveredBody,
        Msg::Ok,
    ];
}

/// Resolve a message in a language.
#[must_use]
pub const fn tr(lang: Lang, msg: Msg) -> &'static str {
    match lang {
        Lang::En => en(msg),
        Lang::Ru => ru(msg),
    }
}

const fn en(msg: Msg) -> &'static str {
    match msg {
        Msg::Tools => "Tools",
        Msg::Place => "Place",
        Msg::Erase => "Erase",
        Msg::Paint => "Paint",
        Msg::Eyedropper => "Eyedropper",
        Msg::BoxTool => "Box (2 clicks)",
        Msg::Sphere => "Sphere",
        Msg::FloodFill => "Flood fill",
        Msg::Select => "Select",
        Msg::Radius => "Radius",
        Msg::Colour => "Colour",
        Msg::ModelColours => "Colours in model",
        Msg::Mirror => "Mirror",
        Msg::Pivot => "Pivot",
        Msg::CenterPivot => "Center",
        Msg::Size => "Size",
        Msg::Crop => "Crop to content",
        Msg::Resize => "Resize",
        Msg::Grow => "Grow",
        Msg::Voxels => "Voxels",
        Msg::Reference => "Reference",
        Msg::Side => "Side",
        Msg::Depth => "Depth",
        Msg::Flip => "Flip",
        Msg::Move => "Move",
        Msg::Show => "Show",
        Msg::Remove => "Remove",
        Msg::Selected => "Selected",
        Msg::Delete => "Delete",
        Msg::Copy => "Copy",
        Msg::Paste => "Paste",
        Msg::Views => "Views",
        Msg::Front => "Front",
        Msg::Back => "Back",
        Msg::Left => "Left",
        Msg::Right => "Right",
        Msg::Top => "Top",
        Msg::Bottom => "Bottom",
        Msg::File => "File",
        Msg::New => "New",
        Msg::OpenKv6 => "Open .kv6…",
        Msg::OpenVox => "Open .vox…",
        Msg::OpenProject => "Open project…",
        Msg::OpenReference => "Open reference image…",
        Msg::Save => "Save",
        Msg::SaveAs => "Save As…",
        Msg::ExportKv6 => "Export .kv6…",
        Msg::ExportVxl => "Export .vxl…",
        Msg::ExportVox => "Export .vox…",
        Msg::Edit => "Edit",
        Msg::Undo => "Undo",
        Msg::Redo => "Redo",
        Msg::View => "View",
        Msg::Lighting => "Lighting",
        Msg::Grid => "Reference grid",
        Msg::VoxelEdges => "Voxel edges",
        Msg::Render => "Render",
        Msg::RenderSprite => "Sprite",
        Msg::RenderVoxel => "Voxel grid",
        Msg::Language => "Language",
        Msg::HelpApply => "LMB: apply tool",
        Msg::HelpOrbit => "RMB: orbit · MMB/Shift+RMB: pan · Home: recenter · wheel: zoom",
        Msg::HelpSelect => {
            "drag selected: move · drag empty: marquee · Shift/Alt +/- · Ctrl+click pick"
        }
        Msg::Untitled => "untitled",
        Msg::ConfirmQuitTitle => "Unsaved changes",
        Msg::ConfirmQuitBody => "Quit without saving?",
        Msg::QuitAnyway => "Quit",
        Msg::Cancel => "Cancel",
        Msg::Saving => "Saving…",
        Msg::RecoveredTitle => "Recovered work",
        Msg::RecoveredBody => "Loaded unsaved work from an autosave. Save it to keep it.",
        Msg::Ok => "OK",
    }
}

const fn ru(msg: Msg) -> &'static str {
    match msg {
        Msg::Tools => "Инструменты",
        Msg::Place => "Поставить",
        Msg::Erase => "Стереть",
        Msg::Paint => "Покрасить",
        Msg::Eyedropper => "Пипетка",
        Msg::BoxTool => "Куб (2 клика)",
        Msg::Sphere => "Сфера",
        Msg::FloodFill => "Заливка",
        Msg::Select => "Выделение",
        Msg::Radius => "Радиус",
        Msg::Colour => "Цвет",
        Msg::ModelColours => "Цвета модели",
        Msg::Mirror => "Зеркало",
        Msg::Pivot => "Опорная точка",
        Msg::CenterPivot => "По центру",
        Msg::Size => "Размер",
        Msg::Crop => "Обрезать по содержимому",
        Msg::Resize => "Изменить размер",
        Msg::Grow => "Расширить",
        Msg::Voxels => "Воксели",
        Msg::Reference => "Опора",
        Msg::Side => "Сбоку",
        Msg::Depth => "Глубина",
        Msg::Flip => "Отразить",
        Msg::Move => "Двигать",
        Msg::Show => "Показать",
        Msg::Remove => "Убрать",
        Msg::Selected => "Выделено",
        Msg::Delete => "Удалить",
        Msg::Copy => "Копировать",
        Msg::Paste => "Вставить",
        Msg::Views => "Виды",
        Msg::Front => "Спереди",
        Msg::Back => "Сзади",
        Msg::Left => "Слева",
        Msg::Right => "Справа",
        Msg::Top => "Сверху",
        Msg::Bottom => "Снизу",
        Msg::File => "Файл",
        Msg::New => "Создать",
        Msg::OpenKv6 => "Открыть .kv6…",
        Msg::OpenVox => "Открыть .vox…",
        Msg::OpenProject => "Открыть проект…",
        Msg::OpenReference => "Открыть опорное изображение…",
        Msg::Save => "Сохранить",
        Msg::SaveAs => "Сохранить как…",
        Msg::ExportKv6 => "Экспорт .kv6…",
        Msg::ExportVxl => "Экспорт .vxl…",
        Msg::ExportVox => "Экспорт .vox…",
        Msg::Edit => "Правка",
        Msg::Undo => "Отменить",
        Msg::Redo => "Повторить",
        Msg::View => "Вид",
        Msg::Lighting => "Освещение",
        Msg::Grid => "Опорная сетка",
        Msg::VoxelEdges => "Грани вокселей",
        Msg::Render => "Рендер",
        Msg::RenderSprite => "Спрайт",
        Msg::RenderVoxel => "Воксельная сетка",
        Msg::Language => "Язык",
        Msg::HelpApply => "ЛКМ: применить инструмент",
        Msg::HelpOrbit => "ПКМ: вращение · СКМ/Shift+ПКМ: пан · Home: в центр · колесо: зум",
        Msg::HelpSelect => {
            "тянуть выделенный: двигать · тянуть пустоту: рамка · Shift/Alt +/- · Ctrl+клик пипетка"
        }
        Msg::Untitled => "без названия",
        Msg::ConfirmQuitTitle => "Несохранённые изменения",
        Msg::ConfirmQuitBody => "Выйти без сохранения?",
        Msg::QuitAnyway => "Выйти",
        Msg::Cancel => "Отмена",
        Msg::Saving => "Сохранение…",
        Msg::RecoveredTitle => "Восстановление",
        Msg::RecoveredBody => {
            "Загружены несохранённые данные из автосейва. Сохраните их, чтобы не потерять."
        }
        Msg::Ok => "OK",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_translates_in_every_language() {
        for lang in Lang::all() {
            for msg in Msg::ALL {
                assert!(!tr(lang, msg).is_empty(), "{lang:?} / {msg:?} is empty");
            }
        }
    }

    #[test]
    fn language_codes_round_trip() {
        for lang in Lang::all() {
            assert_eq!(Lang::from_code(lang.code()), Some(lang));
        }
        assert_eq!(Lang::from_code("RU"), Some(Lang::Ru));
        assert_eq!(Lang::from_code("fr"), None);
    }
}
