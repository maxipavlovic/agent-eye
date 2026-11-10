import type { Store } from '../../store';
import type { ColumnId } from '../../api/types';
import { ago, shortPath, snippetParts } from '../../format';
import { cardProps } from './CardMenu';
import { LANE_LABEL } from './columns';
import { Empty, Sigil, Tok } from './pieces';

/** What a search found: a card per hit with the matching snippet marked. */
export function Results({ store }: { store: Store }) {
  const hits = store.results.value;
  const q = store.query.value.trim();
  return (
    <div class="results">
      <h2 class="rh">
        {hits ? `${hits.length} ${hits.length === 1 ? 'RESULT' : 'RESULTS'}` : 'SEARCHING'} <span>for “{q}”</span>
        {store.searching.value && <i class="dots" />}
      </h2>
      {hits && hits.length === 0 && <Empty id="done" />}
      <div class="rgrid">
        {hits?.map((h, i) => {
          const c = h.card;
          const colId = c.column as ColumnId;
          return (
            <article
              key={c.id}
              class={`card hit${colId === 'needs_you' ? ' need' : ''}`}
              style={`--d:${Math.min(i, 12) * 32}ms`}
              data-id={c.id}
              {...cardProps(c, store)}
            >
              <div class="top">
                <Sigil id={c.id} />
                <h3>{c.title}</h3>
              </div>
              <p class="snip">{snippetParts(h.snippet).map((p, j) => (p.hit ? <mark key={j}>{p.text}</mark> : p.text))}</p>
              <div class="foot">
                <span class={`tag${colId === 'needs_you' ? ' hot' : ''}`}>{LANE_LABEL[colId]?.name ?? String(c.column).toUpperCase()}</span>
                <span class="folder">{shortPath(c.folder)}</span>
                <Tok t={c.tokens} />
                <span class="age">{ago(c.last_activity_at)}</span>
              </div>
            </article>
          );
        })}
      </div>
    </div>
  );
}
