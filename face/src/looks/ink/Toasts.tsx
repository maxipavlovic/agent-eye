import type { Store } from '../../store';

export function Toasts({ store }: { store: Store }) {
  return (
    <div class="toasts" role="status">
      {store.toasts.value.map((t) => (
        <p key={t.id} class={`toast ${t.tone}${t.action ? ' act' : ''}`}>
          {t.text}
          {t.action && <button class="tact" onClick={() => { t.action!.run(); store.dismissToast(t.id); }}>{t.action.label}</button>}
        </p>
      ))}
    </div>
  );
}
