# The pages (Resolve workflow)

Status: **written** (RESOLVE-UI-PLAN RS3). Applies when Settings ▸ General ▸ Workflow is
DaVinci Resolve (the fork's default); the Premiere workflow keeps its header and workspaces.

## What a page is

> id: pages.what | does: the Resolve workflow's pages, one per stage of the work | ui: page.* | cmd: page.* | resolve: pages (Media, Edit, Fusion, Color, Fairlight, Deliver) | premiere: workspaces | when: choosing the Resolve workflow; finding where a tool lives | see: pages.panels

**[Resolve-style]** layout, **[Fork]** mechanism

### Why

Resolve splits the work into pages: bring media in, cut, grade, mix, deliver. Each page puts the
panels for one job on screen and hides the rest, so a beginner always sees a short list of tools
and a pro switches jobs with one key. In FilmCraft a page is a **built-in workspace**: the same
project, the same timeline and one undo history behind every page, only the panel layout changes.

### The window

1. Along the bottom, the **page bar**: the app name on the left, the pages in the middle (an icon
   over each name; the page you are on is lit and underlined in orange-red) and, on the right,
   **Project Manager** ({ui:pageBar.home}, the home screen of [projects.md](projects.md)) and
   **Settings** ({ui:pageBar.settings}, Edit ▸ Preferences ▸ General, `app.settings.general`).
2. Along the top, the menus on a row of their own, then the page's **toolbar**: buttons with a
   label that show or hide that page's panels, the project name in the middle.
3. Click a page in the bar to go there: **Media** ({ui:page.media}), **Edit** ({ui:page.edit}),
   **Fusion** ({ui:page.fusion}), **Color** ({ui:page.color}), **Fairlight**
   ({ui:page.fairlight}), **Deliver** ({ui:page.deliver}). The commands are `page.media`,
   `page.edit`, `page.fusion`, `page.color`, `page.fairlight` and `page.deliver`; with the
   DaVinci Resolve Compatible keys, Shift+2, 4, 6, 7 and 8 open Media, Edit, Color, Fairlight
   and Deliver.

### If you know Resolve

Resolve's Cut page is not shown yet: FilmCraft has no Cut layout (the dual timeline). The
Fusion page holds FilmCraft's much smaller node compositor ([compositor.md](compositor.md)), not
Fusion itself. The Photo page is not planned. Unlike Resolve, the menus
and the pages are one window's furniture: closing a panel on one page doesn't close it on the
others, and every page's layout can be changed and saved (below).

## The pages and their panels

> id: pages.panels | does: which panels each page shows and what Resolve calls them | ui: page.*, pageBar.* | resolve: Media Pool, Inspector, Fairlight mixer | premiere: Project panel, Effect Controls, Audio Track Mixer | when: looking for a panel on a page | see: pages.toolbar

**[Resolve-style]** layouts, built from **[FilmCraft]** panels

Resolve's panel names are used in this workflow (see "Terminology follows the workflow" in
[getting-started.md](getting-started.md)): Media Pool is the Project panel, Inspector is Effect
Controls, Mixer is the Audio Track Mixer.

| Page | Workspace | Layout | Toolbar (left / right) |
|---|---|---|---|
| Media | Media Page | Media Browser and Source Viewer over the Media Pool; Metadata and the meters on the right | Media Browser, Media Pool / Audio Meters, Metadata, Inspector |
| Edit | Edit Page | Media Pool (with Effects and Markers as tabs) down the left; Source Viewer and Timeline Viewer side by side over the Tools strip and the Timeline | Media Pool, Effects, Markers / Quick Export, Mixer, Metadata, Inspector |
| Fusion | Compositing | the Compositor (node canvas with the selected node's settings under it) beside the Timeline Viewer, over the Media Pool and the Timeline ([compositor.md](compositor.md)) | Media Pool, Effects, Compositor / Metadata, Inspector |
| Color | Color Grading | the node grader: Gallery, viewer, Nodes; the clips strip; Primaries and scopes ([color/README.md](color/README.md)) | Gallery, Media Pool, Clips / Nodes, Lumetri Scopes, Effects |
| Fairlight | Fairlight Page | a small viewer and the meters with the loudness readout across the top; Tools and the Timeline; the Mixer (and Essential Sound) on the right | Media Pool, Effects, Markers / Mixer, Audio Meters, Metadata, Inspector |
| Deliver | Deliver Page | Render Settings down the left; the Timeline Viewer and the Render Queue over the Timeline | Render Settings / Render Queue |

Differences from Resolve: the Edit page keeps FilmCraft's narrow Tools strip beside the timeline
(Resolve puts the tools in a bar over it); the Fairlight page's top strip shows the Mix meters,
not one meter per track (the per-track meters are in the Mixer); Resolve's Index, Sound Library,
Keyframes, ADR, LUTs and Lightbox have no FilmCraft panel yet, so their buttons are left out.

## Show and hide panels from the toolbar

> id: pages.toolbar | does: the page toolbar shows and hides that page's extra panels | ui: pageToolbar.* | resolve: interface toolbar | premiere: Window menu | when: a panel you need isn't on the page | see: pages.layout

**[Resolve-style]**

### Why

A page starts with the panels most people need for that job. The toolbar adds the rest when you
need them and takes them away again when you need the room, without hunting through Window.

### Steps

1. On the Edit page, click **Inspector** in the toolbar ({ui:pageToolbar.EffectControls}). It
   opens as a column on the right; the button is lit while the panel is on screen.
2. Click **Inspector** again to hide it.
3. Click **Media Pool** ({ui:pageToolbar.Project}) to hide the Media Pool and give the viewers
   and the timeline the whole width; click again to bring it back on the left.
4. A button whose panel is a hidden tab (Effects behind the Media Pool) brings that tab forward.
5. **Mixer** ({ui:pageToolbar.AudioTrackMixer}) opens the Mixer on the right of the Edit page,
   the way Resolve does.

A button on the left of the toolbar opens its panel on the left of the page, one on the right
opens it on the right. Hiding the last panel of a page is refused (the status bar says why).

## Deliver: render settings and the render queue

> id: pages.deliver | does: render settings and a render queue on the Deliver page | ui: export.*, page.deliver | cmd: export.queue.start, file.exportMedia, page.deliver | resolve: Deliver page render settings and queue | premiere: Export mode, Adobe Media Encoder queue | when: turning the timeline into files, several at once | see: pages.what

**[Resolve-style]** layout, **[FilmCraft]** export settings

### Why

The Deliver page is where a finished timeline becomes a file. The settings are FilmCraft's export
settings ([export.md](export.md)), the same ones File ▸ Export ▸ Media shows; in the Resolve
workflow that menu item (and Ctrl+M, `mode.export`) opens the Deliver page.

### Steps

1. Click **Deliver** in the page bar ({ui:page.deliver}).
2. In **Render Settings** ({ui:pageToolbar.RenderSettings} shows or hides it) set the File Name
   ({ui:export.fileName}), the Location and a Preset ({ui:export.preset}).
3. Choose the **Range** under the settings ({ui:export.range}): the entire timeline, In to Out,
   work area or custom.
4. Click **Add to Render Queue** ({ui:export.addToQueue}) to queue it, or **Render**
   ({ui:export.render}) to export now (`file.exportMedia`).
5. The **Render Queue** on the right ({ui:pageToolbar.RenderQueue}) lists the jobs; **Start
   Render** ({ui:export.queue.start}) works through them (`export.queue.start`).

If you know Premiere: this is the Export mode's Media File settings and queue, arranged around
the Timeline Viewer; the Summary box is replaced by the estimated file size under the settings.

## Rearrange a page and keep it

> id: pages.layout | does: moves and resizes panels on a page and saves or resets the layout | cmd: window.workspace.saveChanges, window.workspace.reset, window.workspace.delete | resolve: Workspace ▸ Reset UI Layout | premiere: Workspaces ▸ Save Changes / Reset | when: making a page fit your screen and habits | see: pages.toolbar

**[FilmCraft]** workspaces, **[Fork]** page layouts

### Why

The page layouts are starting points. Drag a panel's tab to move it, drag a gutter to resize, and
save the result so the page opens your way every time.

### Steps

1. Change the page: drag tabs and gutters, or use the toolbar.
2. Choose Window ▸ Workspaces ▸ **Save Changes to this Workspace**
   (`window.workspace.saveChanges`). The page's workspace (for example "Edit Page") keeps the
   new layout.
3. To go back to the original, choose Window ▸ Workspaces ▸ **Edit Workspaces…**, select the
   page's workspace and delete its saved changes (`window.workspace.delete`), or **Reset to
   Saved Layout** (`window.workspace.reset`) to undo unsaved changes.

In the Resolve workflow Window ▸ Workspaces lists the pages' workspaces (Media Page, Edit Page,
Fairlight Page, Deliver Page, Color Grading) instead of Assembly, Editing, Audio and Color;
choosing one of those names by command or key opens its page. Back in the Premiere workflow the
page workspaces are not listed and the Premiere workspaces are exactly as they were. Switching
the workflow keeps each workflow's layout as you left it, unsaved changes included, and puts it
back when you return. A workspace of your own saved before this release under one of the page
names (for example "Edit Page") is renamed "Edit Page (custom)" once, so it stays in the list.
In this workflow the Import mode (`mode.import`) opens the Media page.
