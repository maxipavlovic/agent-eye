import type { Look } from './look';

/** A look with nothing drawn, for tests of shared parts outside a real look. */
export const TEST_LOOK: Look = {
  name: 'test',
  art: {
    lane: (id) => <i data-art={`lane-${id}`} />,
    icon: (name) => <i data-art={name} />,
    mark: (s, _scale, label) => <i data-art={`mark-${s}`} aria-label={label} />,
    sigil: () => null,
    empty: () => null,
  },
  Header: () => <header />,
  ProjectBar: () => <nav />,
  Head: ({ col }) => <h2>{col.title}</h2>,
  Welcome: () => <main />,
  Loading: () => <div />,
};
