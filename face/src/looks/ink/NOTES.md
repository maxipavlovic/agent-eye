# ink: one-bit paper, rearranged
- Concept: same one-bit paper board, but quiet cards and space by importance. Cards show only the ask (Needs you), one state line (Running), one next line (Waiting), Start (Ideas), title + age (Paused/Done).
- Layouts (header switch, remembered): Lanes = weighted lanes (Needs you 2fr, Running/Waiting 1.2fr, Paused 1fr, Ideas .8fr); Grid = bento (Needs you 2-3 col grid, Running/Waiting/Ideas stacked right, Paused as tiles grouped by project).
- Done is folded to a tab/bar with its count by default; click to unfold (remembered in localStorage).
- Project bar: the top 5 projects plus other, with needs-you counts (`model.ts`); tags show on cards only while All is selected.
- Palette/fonts/art pixel: paper #f2efe6, ink #161616, vermilion #ff4a1c; Silkscreen labels, Inter text, Doto counts; 2 px art pixel.
- A board with no sessions, no ideas and an empty archive shows `Welcome` instead of empty lanes: the pixel eye, a pointer to the enabled agents (names from `/api/agents`) and a note that `eye` works in the terminal.
