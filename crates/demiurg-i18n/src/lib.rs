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
    MovePivot,
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
    Opacity,
    // Selection
    Selected,
    Delete,
    Copy,
    Cut,
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
    NewRig,
    ConvertToRig,
    Open,
    OpenRecent,
    ClearRecent,
    OpenReference,
    PasteReference,
    Save,
    SaveAs,
    ExportKv6,
    ExportVxl,
    ExportVox,
    ExportCharacter,
    Rig,
    Bones,
    AddBone,
    AxisJoint,
    DummyRoot,
    DuplicateBone,
    ExtractToBone,
    DeleteBone,
    MoveBoneUp,
    MoveBoneDown,
    Sculpt,
    Skeleton,
    Animate,
    Clips,
    Play,
    Pause,
    PrevKey,
    NextKey,
    AddKey,
    DeleteKey,
    AddClip,
    DeleteClip,
    Parent,
    Joint,
    Axis,
    Edit,
    Undo,
    Redo,
    View,
    Lighting,
    Grid,
    VoxelEdges,
    FlipX,
    Render,
    RenderSprite,
    RenderVoxel,
    Language,
    // Help line
    HelpApply,
    HelpOrbit,
    HelpSelect,
    // Animate-mode posing hints (bottom of the clip panel)
    PoseHint,
    PoseNeedKey,
    PoseUnposeable,
    // Animate timeline: per-clip playback
    Loop,
    Length,
    Translation,
    Rotation,
    Scale,
    GizmoHint,
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
    pub const ALL: [Msg; 114] = [
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
        Msg::MovePivot,
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
        Msg::Opacity,
        Msg::Selected,
        Msg::Delete,
        Msg::Copy,
        Msg::Cut,
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
        Msg::NewRig,
        Msg::ConvertToRig,
        Msg::Open,
        Msg::OpenRecent,
        Msg::ClearRecent,
        Msg::OpenReference,
        Msg::PasteReference,
        Msg::Save,
        Msg::SaveAs,
        Msg::ExportKv6,
        Msg::ExportVxl,
        Msg::ExportVox,
        Msg::ExportCharacter,
        Msg::Rig,
        Msg::Bones,
        Msg::AddBone,
        Msg::AxisJoint,
        Msg::DummyRoot,
        Msg::DuplicateBone,
        Msg::ExtractToBone,
        Msg::DeleteBone,
        Msg::MoveBoneUp,
        Msg::MoveBoneDown,
        Msg::Sculpt,
        Msg::Skeleton,
        Msg::Animate,
        Msg::Clips,
        Msg::Play,
        Msg::Pause,
        Msg::PrevKey,
        Msg::NextKey,
        Msg::AddKey,
        Msg::DeleteKey,
        Msg::AddClip,
        Msg::DeleteClip,
        Msg::Parent,
        Msg::Joint,
        Msg::Axis,
        Msg::Edit,
        Msg::Undo,
        Msg::Redo,
        Msg::View,
        Msg::Lighting,
        Msg::Grid,
        Msg::VoxelEdges,
        Msg::FlipX,
        Msg::Render,
        Msg::RenderSprite,
        Msg::RenderVoxel,
        Msg::Language,
        Msg::HelpApply,
        Msg::HelpOrbit,
        Msg::HelpSelect,
        Msg::PoseHint,
        Msg::PoseNeedKey,
        Msg::PoseUnposeable,
        Msg::Loop,
        Msg::Length,
        Msg::Translation,
        Msg::Rotation,
        Msg::Scale,
        Msg::GizmoHint,
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

#[allow(clippy::too_many_lines)] // an exhaustive string catalogue, one arm per Msg
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
        Msg::MovePivot => "Move pivot",
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
        Msg::Opacity => "Opacity",
        Msg::Selected => "Selected",
        Msg::Delete => "Delete",
        Msg::Copy => "Copy",
        Msg::Cut => "Cut",
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
        Msg::NewRig => "New rig",
        Msg::ConvertToRig => "Convert to rig",
        Msg::Open => "Open…",
        Msg::OpenRecent => "Open recent",
        Msg::ClearRecent => "Clear recent",
        Msg::OpenReference => "Open reference image…",
        Msg::PasteReference => "Paste image (Ctrl+V)",
        Msg::Save => "Save",
        Msg::SaveAs => "Save As…",
        Msg::ExportKv6 => "Export .kv6…",
        Msg::ExportVxl => "Export .vxl…",
        Msg::ExportVox => "Export .vox…",
        Msg::ExportCharacter => "Export character…",
        Msg::Rig => "Rig",
        Msg::Bones => "Bones",
        Msg::AddBone => "Add bone",
        Msg::AxisJoint => "3-axis joint",
        Msg::DummyRoot => "Dummy root",
        Msg::DuplicateBone => "Duplicate bone",
        Msg::ExtractToBone => "Extract to bone",
        Msg::DeleteBone => "Delete bone",
        Msg::MoveBoneUp => "Move bone up",
        Msg::MoveBoneDown => "Move bone down",
        Msg::Sculpt => "Sculpt",
        Msg::Skeleton => "Skeleton",
        Msg::Animate => "Animate",
        Msg::Clips => "Clips",
        Msg::Play => "Play",
        Msg::Pause => "Pause",
        Msg::PrevKey => "Previous keyframe",
        Msg::NextKey => "Next keyframe",
        Msg::AddKey => "Add key",
        Msg::DeleteKey => "Delete key",
        Msg::AddClip => "Add clip",
        Msg::DeleteClip => "Delete clip",
        Msg::Parent => "Parent",
        Msg::Joint => "Joint",
        Msg::Axis => "Axis",
        Msg::Edit => "Edit",
        Msg::Undo => "Undo",
        Msg::Redo => "Redo",
        Msg::View => "View",
        Msg::Lighting => "Lighting",
        Msg::Grid => "Reference grid",
        Msg::VoxelEdges => "Voxel edges",
        Msg::FlipX => "Flip X",
        Msg::Render => "Render",
        Msg::RenderSprite => "Sprite",
        Msg::RenderVoxel => "Voxel grid",
        Msg::Language => "Language",
        Msg::HelpApply => "LMB: apply tool",
        Msg::HelpOrbit => "RMB: orbit · MMB/Shift+RMB: pan · Home: recenter · wheel: zoom",
        Msg::HelpSelect => {
            "drag selected: move · drag empty: marquee · Shift/Alt +/- · Ctrl+click pick"
        }
        Msg::PoseHint => "click a bone, then left-drag to rotate it into the key",
        Msg::PoseNeedKey => "select a keyframe to pose",
        Msg::PoseUnposeable => "this bone can't be posed (root or locked)",
        Msg::Loop => "loop",
        Msg::Length => "length",
        Msg::Translation => "move",
        Msg::Rotation => "rotate",
        Msg::Scale => "scale",
        Msg::GizmoHint => "gizmo:",
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

#[allow(clippy::too_many_lines)] // an exhaustive string catalogue, one arm per Msg
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
        Msg::MovePivot => "Двигать опору",
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
        Msg::Opacity => "Непрозрачность",
        Msg::Selected => "Выделено",
        Msg::Delete => "Удалить",
        Msg::Copy => "Копировать",
        Msg::Cut => "Вырезать",
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
        Msg::NewRig => "Новый риг",
        Msg::ConvertToRig => "Преобразовать в риг",
        Msg::Open => "Открыть…",
        Msg::OpenRecent => "Открыть недавние",
        Msg::ClearRecent => "Очистить недавние",
        Msg::OpenReference => "Открыть опорное изображение…",
        Msg::PasteReference => "Вставить из буфера (Ctrl+V)",
        Msg::Save => "Сохранить",
        Msg::SaveAs => "Сохранить как…",
        Msg::ExportKv6 => "Экспорт .kv6…",
        Msg::ExportVxl => "Экспорт .vxl…",
        Msg::ExportVox => "Экспорт .vox…",
        Msg::ExportCharacter => "Экспорт персонажа…",
        Msg::Rig => "Риг",
        Msg::Bones => "Кости",
        Msg::AddBone => "Добавить кость",
        Msg::AxisJoint => "3-осевой сустав",
        Msg::DummyRoot => "Фиктивный корень",
        Msg::DuplicateBone => "Дублировать кость",
        Msg::ExtractToBone => "Вытащить в кость",
        Msg::DeleteBone => "Удалить кость",
        Msg::MoveBoneUp => "Переместить кость вверх",
        Msg::MoveBoneDown => "Переместить кость вниз",
        Msg::Sculpt => "Лепка",
        Msg::Skeleton => "Скелет",
        Msg::Animate => "Анимация",
        Msg::Clips => "Клипы",
        Msg::Play => "Воспроизвести",
        Msg::Pause => "Пауза",
        Msg::PrevKey => "Предыдущий ключевой кадр",
        Msg::NextKey => "Следующий ключевой кадр",
        Msg::AddKey => "Добавить ключ",
        Msg::DeleteKey => "Удалить ключ",
        Msg::AddClip => "Добавить клип",
        Msg::DeleteClip => "Удалить клип",
        Msg::Parent => "Родитель",
        Msg::Joint => "Сустав",
        Msg::Axis => "Ось",
        Msg::Edit => "Правка",
        Msg::Undo => "Отменить",
        Msg::Redo => "Повторить",
        Msg::View => "Вид",
        Msg::Lighting => "Освещение",
        Msg::Grid => "Опорная сетка",
        Msg::VoxelEdges => "Грани вокселей",
        Msg::FlipX => "Отразить по X",
        Msg::Render => "Рендер",
        Msg::RenderSprite => "Спрайт",
        Msg::RenderVoxel => "Воксельная сетка",
        Msg::Language => "Язык",
        Msg::HelpApply => "ЛКМ: применить инструмент",
        Msg::HelpOrbit => "ПКМ: вращение · СКМ/Shift+ПКМ: пан · Home: в центр · колесо: зум",
        Msg::HelpSelect => {
            "тянуть выделенный: двигать · тянуть пустоту: рамка · Shift/Alt +/- · Ctrl+клик пипетка"
        }
        Msg::PoseHint => "кликните кость, затем тяните ЛКМ, чтобы повернуть её в кадре",
        Msg::PoseNeedKey => "выберите кадр для позирования",
        Msg::PoseUnposeable => "эту кость нельзя позировать (корень или заблокирована)",
        Msg::Loop => "цикл",
        Msg::Length => "длина",
        Msg::Translation => "сдвиг",
        Msg::Rotation => "поворот",
        Msg::Scale => "масштаб",
        Msg::GizmoHint => "гизмо:",
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
