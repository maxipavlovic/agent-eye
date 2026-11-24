# iris: the board as a technical drawing

- Concept: the board is a drawing of your work. One sheet in a hairline frame, a row of header cells, full-height lane lines, a title block (how many are open and done, how much is archived, when it was drawn, the look's code IR–1).
- Line: every rule, frame and drawing is the thinnest line the browser draws (`--hair` 1px, .5px on Retina, .34px at 3x; `--sw` for SVG strokes). Drawings and Safari's borders are one device pixel; Chrome rounds a border up to one CSS pixel. Hierarchy comes from contrast (`--rule` vs `--rule-2`), never from thicker lines.
- Type: Archivo Extra Condensed 800 for counts (Needs you the biggest, in the accent), Archivo Expanded capitals for labels, Inter for everything read (17 px titles 600, 16 px text 400), Martian Mono only for codes.
- Colour: grey paper `#e8e8e4` / night `#0b0b0a`, ink, and one accent only for what needs you (the eight presets, tuned per theme; `tokens.test.ts` keeps each readable).
- Lane head: count and the lane's object drawn with one line (bulb, tape reel that turns while something works, bell, hourglass, moon, flag), the name, one fact (`facts.ts`) and a timeline: one tick per session on a log scale, two weeks ago to now.
- Cards: the fixed frame of every look; no stamp per session. Marks hang from the card's top rule; a session whose agent is stopped waiting for you has its top rule in the accent and its `!` blinks; a working one has a short stroke travelling along its top rule. The open card gets four corner marks.
