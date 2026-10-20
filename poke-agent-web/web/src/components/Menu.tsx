import { useEffect, useRef, useState, type ReactNode } from 'react';

/** The header's ☰: everything a viewer looks at once rather than all the time. */
export function Menu({ children }: { children: (close: () => void) => ReactNode }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const away = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setOpen(false);
    };
    window.addEventListener('pointerdown', away);
    window.addEventListener('keydown', escape);
    return () => {
      window.removeEventListener('pointerdown', away);
      window.removeEventListener('keydown', escape);
    };
  }, [open]);

  return (
    <div className="menu" ref={root}>
      <button
        className={`menu-button${open ? ' on' : ''}`}
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        aria-haspopup="menu"
        aria-label="menu"
        title="menu"
      >
        ☰
      </button>
      {open && (
        <div className="menu-panel" role="menu">
          {children(() => setOpen(false))}
        </div>
      )}
    </div>
  );
}
