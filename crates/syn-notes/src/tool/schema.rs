//! Описание и JSON-схема инструмента `notes` (перенесены из каталога synthos).

use serde_json::json;

pub const KEY_NOTES: &str = "notes";

/// Краткая подпись инструмента.
pub const SUMMARY: &str = "the user's Notes: pages, kanban tasks, Gantt, calendar, mind maps, charts; agenda and journal.";

/// Описание для модели.
pub const DESCRIPTION: &str = "Read and edit the user's Notes (the Notes mode of \
                the app): a tree of markdown pages — a free canvas where \
                blocks have coordinates — with shapes/arrows, kanban boards \
                and Gantt charts embedded in them. Changes appear in the UI \
                at once and are saved automatically. LIFE MANAGEMENT: agenda \
                (the day at a glance: overdue, due today / tomorrow / soon, \
                events, recently done, board counts — ONE call, use it first \
                for \"what should I do\" questions), tasks (cards across all \
                boards filtered by due/tag/priority/done/board/column/query), \
                log (history: what was added, moved, done, archived, when and \
                by whom — the app records every change itself, never keep a \
                history page by hand), journal (the day's page: get or create \
                Journal / yyyy-mm / yyyy-mm-dd, append notes). Cards carry \
                created/done dates, repeat (daily…yearly: closing spawns the \
                next one), attachments (attach path|attachment → image \
                thumbnail or file with a paperclip), and every board has a \
                DONE column (done=true on a column): move_card there = task \
                done; done cards auto-archive after archive_after days. Due \
                dates already appear in the calendar — never copy them into \
                events. TIME: every reply opens with a `now:` line — the \
                local date, clock time and weekday at the moment of the \
                call; your system prompt carries the date alone. Take due \
                dates, event and reminder times and journal stamps from it \
                instead of guessing the clock, and when you need the time \
                before writing anything, read first (agenda is the cheapest \
                call). Actions: list (page \
                tree with ids and the boards/charts on each page), search \
                (titles + text), read (page markdown, layout, its \
                boards/charts with card ids, links; blocks=true lists blocks \
                with coordinates; pages=[…], depth=all or page=\"all\" read \
                many pages, a whole subtree or the whole project in ONE \
                call), create / update / move / delete / \
                duplicate (pages; update also sets grid/snap, shows or \
                hides the page's right Properties panel and inserts content \
                at a position), open (show a page to the user), \
                attach (file or chat attachment → media block), blocks \
                (op=list | read | insert | set_markdown | delete | move | \
                nest | unnest | set_attrs | pin | unpin — blocks by index, \
                coordinates x y w h, \
                text style color/bg/size/align/weight), shape (op=create | \
                update | delete | connect — rect/ellipse/triangle/diamond, \
                lines and arrows with absolute end points, connect draws an \
                arrow between two pinned blocks), kanban (op=create | read | \
                set_style | add_column | update_column | delete_column | \
                add_card | update_card | move_card | delete_card | delete), \
                gantt (op=create | read | add_task | update_task | \
                delete_task | add_dep | delete_dep | set_zoom | show_today | \
                delete), chart (op=create | read | update | set_data | \
                add_series | update_series | delete_series | set_style | \
                from_table | delete — a line, bar, pie, radar or gauge chart \
                drawn from labels + series of numbers; from_table turns a \
                markdown table on the page into one). Workflow: list → read the pages you need in one \
                call (pages=[…] / depth=all, blocks op=read block=all for \
                every block of a page) → edit. Reading one page or one \
                block per call is the slow way and wastes the user's time. \
                Prefer \
                update with find/replace, mode=append or blocks ops over \
                rewriting a whole page. Pages are addressed by id (12 hex) \
                or exact title; blocks by index from blocks op=list; boards \
                and charts by id, or implicitly when the page has one. \
                PROJECTS. Several projects (.syn files) can be open, a tile \
                each: list, search, agenda, tasks and log cover all of them; \
                any other action \
                works in the project shown in Notes unless you pass \
                project=<title or path>. \
                PROPERTIES PANEL. The right panel of the Notes mode \
                (Properties | Links tabs) is remembered by each page: \
                update props_panel=hidden|shown sets it, read shows it in \
                the layout line, and the screen follows at once when that \
                page is open. For every page at once: update page=\"all\" \
                props_panel=hidden — ONE call; pages=[…] takes a list. \
                NESTING. What a toggle hides — and what a callout, a quote or \
                a list item holds — are its nested blocks, written in markdown \
                as quote lines: EVERY line of the content, table rows \
                included, starts with \"> \". A \"> [!toggle] Payment \
                schedule\" line followed by a bare table is two blocks side by \
                side, and the toggle then collapses nothing. Write the whole \
                thing as quote lines (\"> [!toggle] Payment schedule\", \">\", \
                \"> | # | Date |\", \"> | --- | --- |\", \"> | 1 | 2025-06 |\"), \
                or fix a page that already has them apart with blocks op=nest \
                (the block goes inside the one above it). blocks op=list shows \
                nested blocks as \"#3.0\" under their container. \
                LAYOUT. A new page is a free canvas, but as long as none of \
                its blocks is pinned they flow in one centred column and \
                the page reads as a plain document — keep it that way and \
                pass no coordinates unless the user asked for a canvas. \
                The moment anything is pinned (a shape, a board, a chart, \
                blocks op=pin, insert with x/y) nothing arranges the rest \
                for you: EVERY block must then carry x, y and w (plus h for \
                shapes, media, boards, charts, mind maps and calendars), \
                otherwise the unplaced ones are drawn in the flow column ON \
                TOP of the pinned blocks and the page turns into a pile. \
                create layout=free with content, and update layout=free, \
                pin the unplaced blocks in a column where the flow had \
                them; content added later carries no geometry, so after \
                such an edit on a page with pinned blocks run blocks \
                op=arrange once (stacks every unplaced block below the \
                pinned ones) or pin them one by one (blocks op=pin x y w). \
                Plan the layout before writing it — e.g. one column at x=40 \
                with y growing by the block's height + 24, objects below \
                the text — and check the result with read blocks=true, \
                where h=~ marks a height the editor estimated; a reply line \
                starting with !! free layout lists blocks still unplaced on \
                a page with pinned ones, and !! overlaps names blocks whose \
                frames intersect. !! gap N px above it (in the block list) \
                marks empty space over a block: close it by moving that \
                block up — a heading brings the blocks of its section along.";

/// Схема `notes`: свойств много, и один `json!` на всё упирается в лимит
/// рекурсии макроса — собираем карту из мелких литералов.
pub fn notes_schema() -> serde_json::Value {
    let props: Vec<(&str, serde_json::Value)> = vec![
        ("action", json!({
            "type": "string",
            "enum": ["list", "search", "read", "create", "update", "move",
                     "delete", "duplicate", "open", "attach", "blocks", "shape",
                     "kanban", "gantt", "mindmap", "calendar", "chart",
                     "agenda", "tasks", "log", "journal"],
            "description": "What to do. blocks, shape, kanban, gantt, mindmap, \
                calendar and chart take the sub-operation in op. agenda = \
                the day at a glance (overdue, due today/tomorrow/soon, events, \
                recently done, boards) in ONE call — start here for any \
                \"what should I do / how is my day / week\" question; tasks \
                = cards across all boards with filters (due, tag, priority, \
                done, board, column, query); log = what changed and when \
                (cards moved/done/added, events, pages; who did it); journal \
                = the page of a day (get or create, append content)."
        })),
        ("project", json!({
            "type": "string",
            "description": "Which notes project (.syn file) to work in: the \
                title of an open project as list shows it (file name without \
                .syn) or the path to a .syn file — one that is not open yet \
                opens as a new tile. Default: the project shown in Notes. \
                list, search, agenda, tasks and log without project cover \
                EVERY open project; \
                page and board ids from another project find their project by \
                themselves."
        })),
        ("page", json!({
            "type": "string",
            "description": "Page: id (12 hex, from list) or exact title; \
                for duplicate titles use \"Parent / Title\" or the id. \
                Required by read, update, move, delete, duplicate, attach \
                and by kanban/gantt op=create. read also takes \"all\" — \
                every page of the project in one reply; so does update for \
                page settings (props_panel, grid, snap, bg)."
        })),
        ("pages", json!({
            "type": "array",
            "items": { "type": "string" },
            "description": "read: several pages in ONE call (ids or titles) — \
                always prefer this over one call per page. \"all\" as the \
                only item reads the whole project. Everything that fits in \
                one reply comes back; the pages that did not fit are listed \
                at the end by id. update: the same page settings \
                (props_panel, grid, grid_step, snap, snap_step, bg) on every \
                listed page in one call."
        })),
        ("depth", json!({
            "type": "string",
            "description": "read: how many levels of sub-pages to include \
                along with each requested page — \"0\" (default) the page \
                alone, \"1\" its children, \"all\" the whole subtree in one \
                reply. Use it instead of walking the tree page by page."
        })),
        ("title", json!({
            "type": "string",
            "description": "create: page title (made unique among siblings). \
                update: new title. kanban/gantt op=create: optional heading \
                above the object. kanban add_card: the new card's text \
                (required); update_card: new title. \
                mindmap op=create/from_list: text of the root node. chart: \
                heading drawn inside the chart itself."
        })),
        ("content", json!({
            "type": "string",
            "description": "Page markdown for create, update (see mode) and \
                journal (appended to the day page by default). \
                Supported: # headings, - lists, 1. lists, - [ ] todos, > quotes, \
                > [!note] callouts, > [!toggle] toggles, | tables |, ``` code, \
                --- dividers, [[Page title]] wiki links, ![[Page title]] page \
                embeds, ![[shape:rect]]{fill=#4F8CFF} shapes (rect | ellipse | \
                triangle | diamond | line | arrow | arrow2 | curve | curve-arrow | \
                curve-arrow2), heading/callout attributes {color=… bg=… \
                align=center size=22}. Content carries no geometry: on a \
                free-layout page its blocks land in the flow column until you \
                pin them (blocks op=pin x y w)."
        })),
        ("mode", json!({
            "type": "string",
            "enum": ["replace", "append", "prepend", "insert"],
            "description": "update with content: replace the whole page \
                (default; blocks whose markdown changed lose their canvas \
                position and style), or add the fragment at the end / the \
                start / at index|after|before, optionally placed at x y (w h). \
                journal: append (default) | prepend | replace."
        })),
        ("find", json!({
            "type": "string",
            "description": "update: markdown fragment to replace — copy it from \
                read; it may span several blocks, and spacing, line breaks or \
                **/_/` marks may differ. Whole lines with a word or two off are \
                matched to the one clearly closest line(s), named in the result. \
                Must occur once unless all=true."
        })),
        ("replace", json!({
            "type": "string",
            "description": "update: replacement for find (empty string deletes \
                the fragment)."
        })),
        ("all", json!({
            "type": "boolean",
            "description": "update: replace every occurrence of find."
        })),
        ("parent", json!({
            "type": "string",
            "description": "create/move: parent page (id or title); omit or \
                \"root\" for the top level."
        })),
        ("index", json!({
            "type": "integer",
            "description": "create/move (pages): 0-based position among \
                siblings; omit for the end. blocks insert/move, shape create, \
                attach, update mode=append, kanban/gantt op=create: 0-based \
                block index to insert at."
        })),
        ("icon", json!({
            "type": "string",
            "description": "create/update: page icon — an emoji; \"none\" clears it."
        })),
        ("layout", json!({
            "type": "string",
            "enum": ["free"],
            "description": "create/update: every page is a canvas where each \
                block keeps its own x y w h — there is no flow mode. Until \
                something is pinned the blocks flow in one centred column \
                like a document, but a block left without x/y next to \
                pinned ones is drawn over them. Passing layout=free pins \
                the unplaced blocks in one column where the flow had them, \
                so the picture does not change."
        })),
        ("gap", json!({
            "type": "number",
            "description": "blocks op=arrange: spacing between stacked blocks \
                in px (0..400, default 24)."
        })),
        ("only", json!({
            "type": "string",
            "enum": ["flow", "all", "everything"],
            "description": "blocks op=arrange: flow (default) — stack only \
                blocks without coordinates below the pinned ones; all — \
                re-stack every text block in document order, placed objects \
                (calendar, board, chart, mind map, media, shapes) keep their \
                spot; everything — move those too."
        })),
        ("query", json!({
            "type": "string",
            "description": "search: case-insensitive text to find in titles and \
                page markdown. tasks: text to find in card titles and bodies."
        })),
        ("limit", json!({
            "type": "integer",
            "description": "search: max pages to return (default 20). read \
                with pages/depth: max pages to include (all of them by \
                default, as many as fit in one reply). tasks (default 100) \
                and log (default 60): max rows."
        })),
        ("path", json!({
            "type": "string",
            "description": "attach: file on disk to put into the page (image, \
                svg, audio, video or any file)."
        })),
        ("attachment", json!({
            "type": "string",
            "description": "attach: a chat attachment instead of a path — file \
                name, sha256 prefix or \"last\"."
        })),
        ("caption", json!({
            "type": "string",
            "description": "attach: caption / alt text of the media block."
        })),
        ("open", json!({
            "type": "boolean",
            "description": "create / journal: also show the page to the user."
        })),
        ("op", json!({
            "type": "string",
            "description": "blocks: list | read | insert | set_markdown | delete | \
                move | nest (put the block inside the one above it, or into=<block>: \
                a toggle, callout, quote or list item — that is how a table \
                ends up under a toggle) | unnest (take a nested block back out: \
                block=<container>, child=<index> or \"all\") | \
                set_attrs | pin | unpin | arrange (stack blocks in a \
                column on a free page: unplaced ones by default, only=all for \
                every block; x y start the column, w sets the width, gap the \
                spacing). shape: create | update | delete | \
                connect. mindmap: create | read | add_node | update_node | \
                move_node | delete_node | add_link | delete_link | set_layout | \
                set_style | from_list | delete. calendar: create | read | set_view | \
                set_style | add_event | update_event | move_event | delete_event | \
                complete | list_events | add_calendar | update_calendar | \
                delete_calendar | delete. kanban (board=<id> or page=<page \
                with one board> for every op but create): create {page, \
                columns, done_column, title} | read {archived} | set_style \
                {column_width, lane_bg, card_bg, show_counts, archive_after} | \
                add_column {name, color, width, done, keep_hours, \
                move_after_hours, move_to} | update_column {column, name, color, \
                width, done, keep_hours, move_after_hours, move_to} (column \
                timers, counted from the moment a card entered the column: \
                keep_hours — older cards go to the archive; move_after_hours + \
                move_to — they move to that column) | delete_column {column} | add_card \
                {title = the new card's text, column, md, priority, tags, due, \
                duration, start, end, repeat, before} | update_card {card, \
                title, md, priority, tags, due, duration, start, end, repeat, \
                column, before} | schedule {card, date = today | tomorrow | \
                yyyy-mm-dd, time, duration} (put the card on the calendar: \
                start = that day, end = start + duration) | unschedule {card} | \
                move_card {card, column | \
                to_board, before} | delete_card {card} | archive | unarchive \
                {card} (to/from the board's archive) | attach {card, path | \
                attachment, name} | detach {card, file} (a file or image on a \
                card → thumbnail / paperclip) | delete. card is always an \
                EXISTING card (id or exact title); a new card's text goes in \
                title. \
                gantt: create | read | \
                add_task | update_task | delete_task | add_dep | delete_dep | \
                set_zoom | show_today | set_boards {boards} (board cards with a \
                plan become chart rows) | delete. log: filter by action (done | \
                move | add | delete | due | priority | archive | restore | repeat \
                | create | rename)."
        })),
        ("blocks", json!({
            "type": "boolean",
            "description": "read: also list the page blocks with index, kind, \
                canvas coordinates (x y w h; ~ = estimated height) and \
                attributes — needed before blocks/shape ops. Works together \
                with pages/depth, so one call can bring back a whole \
                subtree with its geometry."
        })),
        ("block", json!({
            "type": "string",
            "description": "blocks/shape: the block — its index from blocks \
                op=list (\"3\") or find:<text> that occurs in exactly one block. \
                Re-list after inserting or deleting blocks: indices shift. \
                blocks op=read also takes several at once: \"0,2,5-7\" or \
                \"all\" (every block of the page with its markdown and \
                attributes) — never read blocks one call at a time."
        })),
        ("into", json!({
            "type": "string",
            "description": "blocks op=nest: the block that takes the other one \
                in (index or find:<text>) — a toggle, callout, quote or list \
                item. Omit for the block right above."
        })),
        ("child", json!({
            "type": "string",
            "description": "blocks op=unnest: which nested block to take out — \
                its index inside the container (the \"3.0\" numbering of \
                op=list) or \"all\" (default)."
        })),
        ("md", json!({
            "type": "string",
            "description": "blocks insert: markdown to insert (one or more \
                blocks). blocks set_markdown: the block's new markdown (its \
                coordinates and style are kept). kanban add_card/update_card: \
                card body markdown (a `- [ ]` checklist shows progress)."
        })),
        ("x", json!({
            "type": "number",
            "description": "Canvas x of the block's top-left corner in px (with \
                y): blocks insert/move/pin, shape create/update (frame \
                shapes), update mode=append, attach, kanban/gantt op=create. \
                The page origin is the top-left; a block without x/y flows in \
                the column, so on a free-layout page always pass x, y and w \
                together — an unplaced block ends up over the canvas. Where \
                the placed content ends is in read blocks=true (x y w h per \
                block, h=~ estimated)."
        })),
        ("y", json!({
            "type": "number",
            "description": "Canvas y of the block's top-left corner in px (with \
                x). Leave a gap of ~24 px below the previous block's height so \
                blocks don't overlap."
        })),
        ("w", json!({
            "type": "number",
            "description": "Block width in px (≥ 40; 520 if omitted). Always \
                pass it on a free-layout page — it is what the block's height \
                is estimated from."
        })),
        ("h", json!({
            "type": "number",
            "description": "Block height in px (≥ 20). Required on a \
                free-layout page for shapes, images, boards, charts, mind maps \
                and calendars — without it they get a default 200 px and can \
                sit on top of the next block. Text blocks (paragraphs, \
                headings, lists, tables, callouts, toggles) size themselves \
                and take no h — a folded toggle is one line tall and the \
                blocks below it move down when it unfolds, so never reserve \
                room for its content."
        })),
        ("section", json!({
            "type": "boolean",
            "description": "blocks set_attrs / pin / move of a HEADING to new x/y: \
                its section — the blocks under it in its column up to the \
                next heading of the same or higher level — moves along by \
                the same offset (default). false moves the heading alone."
        })),
        ("attrs", json!({
            "type": "object",
            "additionalProperties": true,
            "description": "blocks set_attrs: attributes to set; null or \"\" \
                clears one. Text: color, bg (#rrggbb), size (6..160), weight \
                (bold|normal), align (left|center|right). Geometry: x y w h \
                (also accepted as plain arguments next to op=set_attrs). \
                Shapes: fill, stroke (#rrggbb or none), sw (0..40), dash \
                (0..60), radius (0..200), opacity (0..100); line points x1 y1 \
                x2 y2 cx1 cy1 cx2 cy2 are relative to the block — prefer \
                shape op=update with absolute points."
        })),
        ("kind", json!({
            "type": "string",
            "description": "shape create/update/connect: rect | ellipse | \
                triangle | diamond | line | arrow | arrow2 (both ends) | curve | \
                curve-arrow | curve-arrow2. Default rect (create) / arrow \
                (connect). chart create/update/from_table: line | bar | pie | \
                radar | gauge (default line, from_table bar). log: card | event \
                | page."
        })),
        ("fill", json!({
            "type": "string",
            "description": "shape: fill color #rrggbb (frame shapes; none = no fill)."
        })),
        ("stroke", json!({
            "type": "string",
            "description": "shape: outline color #rrggbb, none = no outline."
        })),
        ("sw", json!({
            "type": "number",
            "description": "shape: stroke width px (0..40, default 2)."
        })),
        ("dash", json!({
            "type": "number",
            "description": "shape: dash length px (0..60, 0 = solid)."
        })),
        ("radius", json!({
            "type": "number",
            "description": "shape rect: corner radius px (0..200)."
        })),
        ("opacity", json!({
            "type": "number",
            "description": "shape: opacity percent (0..100)."
        })),
        ("x1", json!({
            "type": "number",
            "description": "shape create/update (lines, arrows, curves): start \
                point x in absolute canvas px (with y1). The block frame is \
                computed from the points."
        })),
        ("y1", json!({ "type": "number", "description": "shape: start point y (absolute)." })),
        ("x2", json!({ "type": "number", "description": "shape: end point x (absolute, with y2)." })),
        ("y2", json!({ "type": "number", "description": "shape: end point y (absolute)." })),
        ("cx1", json!({
            "type": "number",
            "description": "shape curves: first control point x (absolute, with \
                cy1); omit for an automatic S-curve."
        })),
        ("cy1", json!({ "type": "number", "description": "shape curves: first control point y." })),
        ("cx2", json!({ "type": "number", "description": "shape curves: second control point x (with cy2)." })),
        ("cy2", json!({ "type": "number", "description": "shape curves: second control point y." })),
        ("from_side", json!({
            "type": "string",
            "description": "shape connect: side of the from block the arrow \
                starts at — auto (facing the other block) | left | right | top \
                | bottom | center."
        })),
        ("to_side", json!({
            "type": "string",
            "description": "shape connect: side of the to block the arrow ends at (same values)."
        })),
        ("grid", json!({
            "type": "string",
            "description": "create/update: canvas grid — none | dots | lines | cross."
        })),
        ("grid_step", json!({
            "type": "number",
            "description": "create/update: grid step px (2..200, default 20)."
        })),
        ("snap", json!({
            "type": "boolean",
            "description": "create/update: snap blocks to the snap step when dragged."
        })),
        ("snap_step", json!({
            "type": "number",
            "description": "create/update: snap step px (1..100, default 5)."
        })),
        ("bg", json!({
            "type": "string",
            "description": "create/update: page background #rrggbb / #rrggbbaa \
                (painted under the grid); none = theme."
        })),
        ("props_panel", json!({
            "type": "string",
            "enum": ["shown", "hidden"],
            "description": "create/update: the right Properties panel of the \
                Notes mode while this page is open — each page remembers its \
                own. With page=\"all\" / pages=[…] update sets it on many \
                pages in one call."
        })),
        ("column_width", json!({
            "type": "number",
            "description": "kanban set_style: default column width px (140..800)."
        })),
        ("lane_bg", json!({
            "type": "string",
            "description": "kanban set_style: column background #rrggbb or \
                #rrggbbaa (translucent tints look best); none = theme."
        })),
        ("card_bg", json!({
            "type": "string",
            "description": "kanban set_style: card background #rrggbb / #rrggbbaa; none = theme."
        })),
        ("show_counts", json!({
            "type": "boolean",
            "description": "kanban set_style: show card counts in column headers."
        })),
        ("zoom", json!({
            "type": "number",
            "description": "gantt set_zoom: px per day (5..90, default 26)."
        })),
        ("map", json!({
            "type": "string",
            "description": "mindmap: map id (mindmap:<id> from list/read). Omit \
                when the page (or the whole project) has exactly one map."
        })),
        ("node", json!({
            "type": "string",
            "description": "mindmap: node id or its exact text; \"root\" is the \
                central node. Ids are in mindmap op=read."
        })),
        ("outline", json!({
            "type": "string",
            "description": "mindmap op=create: markdown to build the map from — a \
                heading or first paragraph becomes the root, nested bullets become \
                nodes (`- [x]` gives a ✓ icon)."
        })),
        ("text", json!({
            "type": "string",
            "description": "mindmap add_node/update_node: node text."
        })),
        ("note", json!({
            "type": "string",
            "description": "mindmap: node note (markdown). calendar: event note."
        })),
        ("link", json!({
            "type": "string",
            "description": "mindmap node / calendar event: page it points to (id \
                or title); \"none\" clears it."
        })),
        ("shape", json!({
            "type": "string",
            "description": "mindmap update_node: node shape — auto | rect | rounded \
                | pill | ellipse | text."
        })),
        ("collapsed", json!({
            "type": "boolean",
            "description": "mindmap update_node: hide the node's children."
        })),
        ("dx", json!({
            "type": "number",
            "description": "mindmap update_node: manual x offset of the node's \
                subtree from the automatic layout (px)."
        })),
        ("dy", json!({
            "type": "number",
            "description": "mindmap update_node: manual y offset (px)."
        })),
        ("direction", json!({
            "type": "string",
            "description": "mindmap create/set_layout: right | left | both | down | radial."
        })),
        ("curve", json!({
            "type": "string",
            "description": "mindmap set_layout: connector shape — bezier | straight | elbow."
        })),
        ("h_gap", json!({
            "type": "number",
            "description": "mindmap set_layout: gap between levels px (8..400, default 48)."
        })),
        ("v_gap", json!({
            "type": "number",
            "description": "mindmap set_layout: gap between sibling nodes px (0..200, default 14)."
        })),
        ("label", json!({
            "type": "string",
            "description": "mindmap add_link: caption of the cross link."
        })),
        ("reset", json!({
            "type": "boolean",
            "description": "mindmap set_layout: drop every manual node offset."
        })),
        ("style", json!({
            "type": "object",
            "additionalProperties": true,
            "description": "mindmap set_style: palette (preset name theme|rainbow|\
                pastel|mono or a list of #rrggbb), node_fill, node_stroke, \
                text_color, line_color, bg, font_size, weight, radius, padding, \
                line_width, line_dash, show_icons, max_node_w. calendar set_style: \
                preset (theme|light|contrast|pastel), event_style (chip|dot|bar), \
                first_weekday (0=Mon), show_week_numbers, hour_from, hour_to, \
                slot_min, compact, font_size, weekend_tint, today_color, header_bg, \
                cell_bg, grid_color, text_color, show_kanban_due, show_gantt. \
                chart set_style: legend (top|bottom|left|right|none), tooltip, \
                animate, grid, x_title, y_title, y_min, y_max (a number or \
                \"auto\"), smooth, points, area (0..1), stacked, horizontal, \
                value_labels, bar_radius, donut (0..0.9), pie_labels \
                (outside|inside|none), percentage, radar_circle, radar_levels, \
                radar_max, gauge_min, gauge_max, needle, ticks, gauge_labels, \
                unit, zones (\"0-50 green, 50-80 orange\")."
        })),
        ("heading", json!({
            "type": "string",
            "description": "calendar op=create: optional heading placed above the \
                widget on the page."
        })),
        ("view", json!({
            "type": "string",
            "description": "calendar create/set_view: year | month | week | day \
                (day is a schedule)."
        })),
        ("anchor", json!({
            "type": "string",
            "description": "calendar create/set_view: date the view is centred on \
                (yyyy-mm-dd, today, tomorrow)."
        })),
        ("calendars", json!({
            "type": "array",
            "items": { "type": "string" },
            "description": "calendar set_view: named calendars the widget shows \
                (ids or names); empty means all."
        })),
        ("categories", json!({
            "type": "array",
            "items": { "type": "string" },
            "description": "chart: labels along the bottom — the slices of a pie, \
                the axes of a radar, the x ticks of a line or bar chart."
        })),
        ("event", json!({
            "type": "string",
            "description": "calendar update_event/move_event/delete_event/complete: \
                event id or exact title (add \"on\" to disambiguate by date)."
        })),
        ("on", json!({
            "type": "string",
            "description": "calendar: the date of the event you mean when several \
                share a title (yyyy-mm-dd). complete, and update_event with done, \
                on a repeating event: the occurrence to mark (default today)."
        })),
        ("date", json!({
            "type": "string",
            "description": "calendar add_event/update_event/move_event: date \
                yyyy-mm-dd, today or tomorrow. journal: the day whose page to \
                open — today (default), yesterday, tomorrow or yyyy-mm-dd."
        })),
        ("end_date", json!({
            "type": "string",
            "description": "calendar: last day of a multi-day event; \"none\" clears it."
        })),
        ("start_time", json!({
            "type": "string",
            "description": "calendar: start time HH:MM in local time; \"none\" makes \
                the event all-day."
        })),
        ("end_time", json!({
            "type": "string",
            "description": "calendar: end time HH:MM (defaults to start + 1 hour)."
        })),
        ("all_day", json!({
            "type": "boolean",
            "description": "calendar: the event takes the whole day (drops the times)."
        })),
        ("done", json!({
            "type": ["boolean", "string"],
            "description": "calendar complete/update_event: mark the event done. \
                A repeating event is marked per occurrence (the one on \"on\", \
                default today): done on Friday leaves Monday open. \
                kanban add_column/update_column: true = this is the board's \
                DONE column — cards moved into it get a done date (and a \
                repeating card spawns its next occurrence). tasks: false \
                (default) lists open cards, true only done ones, \"any\" both."
        })),
        ("repeat", json!({
            "type": "string",
            "description": "calendar and kanban add_card/update_card: none | \
                daily | weekly | monthly | yearly. calendar also takes the \
                shorthands weekdays (= daily, Mon–Fri) and weekends (= daily, \
                Sat–Sun). A repeating card, when moved \
                to the done column, creates the next one with the due date \
                shifted by one period (habits, recurring chores)."
        })),
        ("only_days", json!({
            "type": ["array", "string"],
            "description": "calendar add_event/update_event: keep only these \
                weekdays of a repeating event — mon | tue | wed | thu | fri | \
                sat | sun (1…7 or weekdays/weekends work too). ONE event with \
                repeat=daily, only_days and until replaces a dozen single \
                events; empty or \"all\" clears the filter."
        })),
        ("skip_days", json!({
            "type": ["array", "string"],
            "description": "calendar add_event/update_event: drop these \
                weekdays from a repeating event, e.g. [\"sat\", \"sun\"] on a \
                daily one. The mirror of only_days."
        })),
        ("from", json!({
            "type": "string",
            "description": "gantt add_dep/delete_dep: predecessor task (id or name). \
                mindmap add_link/delete_link: source node. calendar read/list_events: \
                first date of the range (yyyy-mm-dd)."
        })),
        ("to", json!({
            "type": "string",
            "description": "gantt add_dep/delete_dep: successor task (id or name). \
                mindmap add_link/delete_link: target node. calendar read/list_events: \
                last date of the range (yyyy-mm-dd)."
        })),
        ("include_external", json!({
            "type": "boolean",
            "description": "calendar read/list_events: also list board card due \
                dates and Gantt tasks of the project (read-only layer; on by \
                default — the calendar already shows them, never copy a card's \
                due date into an event)."
        })),
        ("board", json!({
            "type": "string",
            "description": "kanban: board id (kanban:<id> from list/read). Omit \
                when the page (or the whole project) has exactly one board."
        })),
        ("calendar", json!({
            "type": "string",
            "description": "calendar: widget id (calendar:<id> from list/read) for \
                read/set_view/set_style/delete, or the named calendar (id or name) \
                for add_event, update_calendar and delete_calendar. Events live in \
                one project-wide store, the widget only picks the view and filter."
        })),
        ("gantt", json!({
            "type": "string",
            "description": "gantt: chart id (gantt:<id> from list/read). Omit \
                when the page (or the whole project) has exactly one gantt chart."
        })),
        ("chart", json!({
            "type": "string",
            "description": "chart: chart id (chart:<id> from list/read). Omit \
                when the page (or the whole project) has exactly one chart."
        })),
        ("series", json!({
            "type": "string",
            "description": "chart update_series/delete_series/set_data: which \
                series — its id or its name. Omit to mean the first one."
        })),
        ("data", json!({
            "type": "array",
            "items": { "type": "number" },
            "description": "chart: the numbers of one series, one per label \
                (create, add_series, update_series, set_data). A gauge takes a \
                single number."
        })),
        ("value", json!({
            "type": "number",
            "description": "chart: one number — with index it replaces that \
                point of the series, alone it is the gauge value."
        })),
        ("table", json!({
            "type": "string",
            "description": "chart create/set_data: the data as a markdown table \
                — the header row names the series, the first column holds the \
                labels, e.g. \"| Month | Plan | Fact |\\n| Jan | 10 | 12 |\". \
                Cells that are not numbers count as 0."
        })),
        ("columns", json!({
            "type": "array",
            "items": { "type": "string" },
            "description": "kanban op=create: column names (default: To do / \
                In progress / Done)."
        })),
        ("column", json!({
            "type": "string",
            "description": "kanban: column by id, name or 1-based number — \
                target of add_card, subject of update_column/delete_column, \
                destination of move_card/update_card. tasks: filter by column \
                name."
        })),
        ("name", json!({
            "type": "string",
            "description": "kanban attach: display name of the attachment \
                (default: the file name). kanban add_column/update_column: column name. \
                gantt add_task/update_task: task name. calendar add_calendar/\
                update_calendar: calendar name. chart add_series/update_series: \
                series name (it labels the legend)."
        })),
        ("color", json!({
            "type": "string",
            "description": "Color of a column, task, calendar, event, mind-map \
                node or chart series: #rrggbb or gray | orange | green | blue | \
                purple | red | teal; \"none\" clears it (a chart series then \
                takes its palette color)."
        })),
        ("width", json!({
            "type": "number",
            "description": "kanban add_column/update_column: column width in \
                px (0 = the board default)."
        })),
        ("card", json!({
            "type": "string",
            "description": "kanban update_card/move_card/delete_card/archive/\
                unarchive/attach/detach: an EXISTING card — id or exact title. \
                Not for add_card: a new card's text goes in title. log: only \
                this card's history."
        })),
        ("priority", json!({
            "type": "string",
            "description": "kanban add_card/update_card: low | medium | high | \
                urgent | none."
        })),
        ("tags", json!({
            "type": "array",
            "items": { "type": "string" },
            "description": "kanban add_card/update_card: tags (array or a \
                comma-separated string)."
        })),
        ("due", json!({
            "type": "string",
            "description": "kanban add_card/update_card: due date yyyy-mm-dd, \
                today, tomorrow or none. tasks: filter — overdue | today | \
                tomorrow | week | month | 14d | none | any | yyyy-mm-dd | \
                from..to."
        })),
        ("duration", json!({
            "type": ["string", "number"],
            "description": "kanban add_card/update_card/schedule: how long the \
                task is expected to take — 1d, 2h, 90m, \"1d 4h\"; a bare \
                number means hours, \"none\" clears it. With a start date it \
                gives the card an end date, so the card shows as a bar in the \
                calendar and on a Gantt chart."
        })),
        ("start", json!({
            "type": "string",
            "description": "kanban add_card/update_card: planned start — \
                yyyy-mm-dd, yyyy-mm-ddThh:mm, today, tomorrow (\"today 10:00\" \
                works too), \"none\" clears it. Together with end (or duration) \
                it puts the card on the calendar as a bar."
        })),
        ("end", json!({
            "type": "string",
            "description": "kanban add_card/update_card: planned end, same \
                formats as start; without it the end comes from duration."
        })),
        ("time", json!({
            "type": "string",
            "description": "kanban schedule: time of day HH:MM for the planned \
                start; without it the card takes whole days."
        })),
        ("boards", json!({
            "type": "array",
            "items": { "type": "string" },
            "description": "calendar set_view: boards whose planned cards the \
                calendar shows as bars (ids, or the page holding the board); \
                empty means every board of the project. gantt set_boards: the \
                same, but empty means only the chart's own tasks."
        })),
        ("since", json!({
            "type": "string",
            "description": "log: start of the range — today | yesterday | week \
                | month | 14d | yyyy-mm-dd | all (default: the last 7 days)."
        })),
        ("until", json!({
            "type": "string",
            "description": "calendar: last date of the repetition (yyyy-mm-dd; \
                \"none\" repeats forever). log: end of the range (yyyy-mm-dd)."
        })),
        ("actor", json!({
            "type": "string",
            "description": "log: user | agent — who made the change."
        })),
        ("days", json!({
            "type": ["integer", "array", "string"],
            "description": "agenda: how many days ahead to include (default 7). \
                calendar add_event/update_event: an alias of only_days (the \
                weekdays of a repeating event)."
        })),
        ("done_days", json!({
            "type": "integer",
            "description": "agenda: show cards done within the last N days \
                (default 3)."
        })),
        ("archived", json!({
            "type": "boolean",
            "description": "kanban read: also list the board's archive. tasks: \
                include archived cards."
        })),
        ("archive_after", json!({
            "type": ["integer", "string"],
            "description": "kanban set_style: days after a card is done before \
                it moves off the board into the archive (0 / none = never)."
        })),
        ("keep_hours", json!({
            "type": ["integer", "string"],
            "description": "kanban add_column/update_column: how long a card may \
                stay in this column — hours (72), or 3d / 12h; after that it \
                leaves the board into the archive (op=unarchive brings it back \
                to the same column). 0 / none / forever = keep forever."
        })),
        ("move_after_hours", json!({
            "type": ["integer", "string"],
            "description": "kanban add_column/update_column: after this many \
                hours in the column (72, 3d) a card moves to the move_to column \
                by itself — e.g. In progress → Overdue. 0 / none = off."
        })),
        ("move_to", json!({
            "type": "string",
            "description": "kanban add_column/update_column: the column (name, id \
                or 1-based number) cards move to after move_after_hours; none \
                clears it."
        })),
        ("done_column", json!({
            "type": "string",
            "description": "kanban create with columns: which column is the \
                DONE one (name or 1-based number). Without it a column named \
                Done / Complete / Finished (or its Russian equivalent) is \
                detected automatically."
        })),
        ("sort", json!({
            "type": "string",
            "description": "tasks: due (default) | priority | created | done."
        })),
        ("tag", json!({
            "type": "string",
            "description": "tasks: only cards carrying this tag."
        })),
        ("file", json!({
            "type": "string",
            "description": "kanban detach: the attachment to remove — its name \
                or asset url from the card line."
        })),
        ("before", json!({
            "type": "string",
            "description": "kanban add_card/move_card/update_card: place the \
                card before this card (id or title) of the target column; omit \
                for the end. Elsewhere (blocks insert/move, shape create, \
                attach, update mode=append, kanban/gantt op=create): insert \
                before this block (index or find:<text>)."
        })),
        ("to_board", json!({
            "type": "string",
            "description": "kanban move_card: move the card to another board (id)."
        })),
        ("task", json!({
            "type": "string",
            "description": "gantt update_task/delete_task: task id or exact name."
        })),
        ("start", json!({
            "type": "string",
            "description": "gantt add_task/update_task: start date yyyy-mm-dd \
                (default today)."
        })),
        ("end", json!({
            "type": "string",
            "description": "gantt add_task/update_task: end date yyyy-mm-dd, \
                inclusive (default start + 2 days; update_task keeps the \
                duration when only start is given)."
        })),
        ("after", json!({
            "type": "string",
            "description": "gantt add_task: make the new task depend on this \
                task (id or name). Elsewhere (blocks insert/move, shape create, \
                attach, update mode=append, kanban/gantt op=create): insert \
                after this block (index or find:<text>)."
        })),
    ];
    let mut map = serde_json::Map::new();
    for (k, v) in props {
        map.insert(k.to_string(), v);
    }
    json!({
        "type": "object",
        "properties": map,
        "required": ["action"],
        "additionalProperties": false
    })
}
