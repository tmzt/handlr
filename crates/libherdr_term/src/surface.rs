//! The tab as herdr last drew it for this client, and one pane cut from it
//! (decision 7; findings 2.3).

use crate::herdr;

/// The tab as flankr last applied it, herdr's own types inside: the last
/// full `PaneSurface` with every later `PaneSurfacePatch` applied in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceView {
    /// The server boot that drew it.
    pub boot_id: String,
    /// herdr's projection revision (focus and topology) behind it.
    pub projection_revision: u64,
    /// The revision of the last full surface or patch applied.
    pub surface_revision: u64,
    /// The whole surface: `frame.width` by `frame.height` cells row-major,
    /// the surface cursor in `frame.cursor`.
    pub frame: herdr::FrameData,
    /// Every pane of the viewed tab, with its rectangles and modes.
    pub panes: Vec<herdr::PaneSurfacePane>,
}

/// One pane of a [`SurfaceView`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneView<'a> {
    pub pane: &'a herdr::PaneSurfacePane,
    /// The pane's terminal cells, one slice per row of its `inner_rect`
    /// (clipped to the surface).
    pub cells: Vec<&'a [herdr::CellData]>,
    /// The cursor, pane-local, when the surface cursor is inside the pane.
    pub cursor: Option<herdr::CursorState>,
}

/// How herdr's view differs from what flankr last asked for, without flankr
/// asking: a change made on the desktop (the 23:32 EDT amendment, item 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopChange {
    /// The pane flankr focused is not in the viewed tab any more.
    PaneNotShown,
    /// Another pane of the tab is focused.
    FocusMoved { focused: Option<String> },
    /// The surface is not the size flankr asked for.
    SurfaceSize { cols: u16, rows: u16 },
    /// The pane's terminal has another number of rows than its rectangle
    /// on flankr's surface: another client holds the tab's geometry.
    PaneRows { pty_rows: u64, shown_rows: u16 },
}

impl SurfaceView {
    /// A view of the full surface `frame`.
    pub fn from_frame(frame: herdr::PaneSurfaceFrame) -> SurfaceView {
        SurfaceView {
            boot_id: frame.boot_id,
            projection_revision: frame.projection_revision,
            surface_revision: frame.surface_revision,
            frame: frame.frame,
            panes: frame.panes,
        }
    }

    /// The surface's width in columns.
    pub fn cols(&self) -> u16 {
        self.frame.width
    }

    /// The surface's height in rows.
    pub fn rows(&self) -> u16 {
        self.frame.height
    }

    /// The pane herdr marks focused in this tab.
    pub fn focused_pane(&self) -> Option<&herdr::PaneSurfacePane> {
        self.panes.iter().find(|p| p.focused)
    }

    /// Apply `patch` (findings 2.3): its cells over the held surface, its
    /// panes over the held entries of the same id, its cursor. A patch whose
    /// base is not the held revision is an error, and nothing is applied.
    pub fn apply_patch(&mut self, patch: herdr::PaneSurfacePatch) -> Result<(), String> {
        if patch.base_surface_revision != self.surface_revision {
            return Err(format!(
                "herdr patch on surface revision {}, held {}",
                patch.base_surface_revision, self.surface_revision
            ));
        }
        let width = usize::from(self.frame.width);
        let height = usize::from(self.frame.height);
        for row in patch.rows {
            let y = usize::from(row.y);
            if y >= height {
                continue;
            }
            for (i, cell) in row.cells.into_iter().enumerate() {
                let x = usize::from(row.x) + i;
                if x >= width {
                    break;
                }
                if let Some(slot) = self.frame.cells.get_mut(y * width + x) {
                    *slot = cell;
                }
            }
        }
        for pane in patch.panes {
            match self.panes.iter_mut().find(|p| p.pane_id == pane.pane_id) {
                Some(held) => *held = pane,
                None => self.panes.push(pane),
            }
        }
        self.frame.cursor = patch.cursor;
        self.boot_id = patch.boot_id;
        self.projection_revision = patch.projection_revision;
        self.surface_revision = patch.surface_revision;
        Ok(())
    }

    /// The pane `pane_id`, cut from the surface by its `inner_rect`, or
    /// `None` when the viewed tab does not show it.
    pub fn pane(&self, pane_id: &str) -> Option<PaneView<'_>> {
        let pane = self.panes.iter().find(|p| p.pane_id == pane_id)?;
        let r = pane.inner_rect;
        let width = usize::from(self.frame.width);
        let height = usize::from(self.frame.height);
        let x0 = usize::from(r.x).min(width);
        let x1 = (usize::from(r.x) + usize::from(r.width)).min(width);
        let y0 = usize::from(r.y).min(height);
        let y1 = (usize::from(r.y) + usize::from(r.height)).min(height);
        let mut cells = Vec::with_capacity(y1 - y0);
        if self.frame.cells.len() == width * height {
            for y in y0..y1 {
                cells.push(&self.frame.cells[y * width + x0..y * width + x1]);
            }
        }
        let cursor = self.frame.cursor.as_ref().and_then(|c| {
            let inside = c.x >= r.x
                && u32::from(c.x) < u32::from(r.x) + u32::from(r.width)
                && c.y >= r.y
                && u32::from(c.y) < u32::from(r.y) + u32::from(r.height);
            inside.then(|| herdr::CursorState {
                x: c.x - r.x,
                y: c.y - r.y,
                visible: c.visible,
                shape: c.shape,
            })
        });
        Some(PaneView {
            pane,
            cells,
            cursor,
        })
    }

    /// The first way this view differs from flankr's last request, `pane_id`
    /// focused on a `cols` by `rows` surface; `None` when it matches. The
    /// caller asks only once herdr has answered flankr's own requests and
    /// drawn after them, so flankr's own changes are not mistaken for the
    /// desktop's.
    pub fn desktop_change(&self, pane_id: &str, cols: u16, rows: u16) -> Option<DesktopChange> {
        let Some(view) = self.pane(pane_id) else {
            return Some(DesktopChange::PaneNotShown);
        };
        if !view.pane.focused {
            return Some(DesktopChange::FocusMoved {
                focused: self.focused_pane().map(|p| p.pane_id.clone()),
            });
        }
        if (self.cols(), self.rows()) != (cols, rows) {
            return Some(DesktopChange::SurfaceSize {
                cols: self.cols(),
                rows: self.rows(),
            });
        }
        if let Some(pty_rows) = view.pty_rows()
            && pty_rows != u64::from(view.pane.inner_rect.height)
        {
            return Some(DesktopChange::PaneRows {
                pty_rows,
                shown_rows: view.pane.inner_rect.height,
            });
        }
        None
    }
}

impl PaneView<'_> {
    /// The pane's size on the surface: its `inner_rect`, the PTY's size
    /// while flankr holds the tab's geometry.
    pub fn size(&self) -> (u16, u16) {
        (self.pane.inner_rect.width, self.pane.inner_rect.height)
    }

    /// The rows of the pane's terminal as herdr reports them
    /// (`scroll.viewport_rows`), which differ from [`PaneView::size`] while
    /// another client holds the tab's geometry.
    pub fn pty_rows(&self) -> Option<u64> {
        self.pane.scroll.map(|s| s.viewport_rows)
    }
}
