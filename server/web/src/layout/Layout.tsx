import { useEffect, useState } from 'react';
import { Link, Outlet, useLocation } from 'react-router';
import { Menu } from 'lucide-react';
import { useEvents } from '../api/events';
import { useHealth } from '../api/queries';
import { Banner } from '../components/Feedback';
import { Button } from '../components/Button';
import { Sidebar } from './Sidebar';
import styles from './Layout.module.css';

export function Layout({ username }: { username: string | null }) {
  const stream = useEvents();
  const [menuOpen, setMenuOpen] = useState(false);
  const location = useLocation();
  const health = useHealth();
  const errors = (health.data ?? []).filter((h) => h.type === 'error');

  // Navigating closes the phone menu.
  useEffect(() => setMenuOpen(false), [location.pathname]);

  useEffect(() => {
    if (!menuOpen) return;
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && setMenuOpen(false);
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [menuOpen]);

  return (
    <div className={styles.shell}>
      <a className={styles.skip} href="#main">
        Skip to content
      </a>
      <header className={styles.mobileBar}>
        <Button
          variant="ghost"
          iconOnly
          label="Open menu"
          icon={<Menu size={20} />}
          onClick={() => setMenuOpen(true)}
          aria-expanded={menuOpen}
          aria-controls="sidebar"
        />
        <Link to="/library" className={styles.mobileBrand}>
          <img src="/favicon.svg" alt="" width={24} height={24} />
          LibriSync
        </Link>
      </header>

      <Sidebar
        id="sidebar"
        open={menuOpen}
        onClose={() => setMenuOpen(false)}
        username={username}
        streamStatus={stream}
        health={health.data ?? []}
      />
      {menuOpen && <div className={styles.scrim} onClick={() => setMenuOpen(false)} aria-hidden />}

      <main id="main" className={styles.main} tabIndex={-1}>
        {errors.length > 0 && location.pathname !== '/system/status' && (
          <div className={styles.healthBanner}>
            <Banner
              tone="danger"
              title={errors.length === 1 ? errors[0]?.message : `${errors.length} problems need attention`}
              action={<Link to="/system/status">View health</Link>}
            />
          </div>
        )}
        <Outlet />
      </main>
    </div>
  );
}
