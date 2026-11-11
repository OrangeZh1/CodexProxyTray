// Each action owns its indicator; background reads never enter this map.
export function createActions(changed = () => {}, { minimumVisibleMs = 0, now = () => performance.now(), wait = ms => new Promise(resolve => setTimeout(resolve, ms)) } = {}) {
  const pending = new Set();
  return {
    has: name => pending.has(name),
    async run(name, operation) {
      if (pending.has(name)) return;
      pending.add(name);
      const started = now();
      changed();
      try { return await operation(); }
      finally {
        const remaining = minimumVisibleMs - (now() - started);
        if (remaining > 0) await wait(remaining);
        pending.delete(name); changed();
      }
    },
  };
}

export function acceptSnapshot(current, next) {
  return !current || next.revision >= current.revision ? next : current;
}

export function systemPreferences(current, snapshot, pending) {
  const incoming={autostart:!!snapshot.autostart,launchTarget:snapshot.launch_target||(snapshot.launch_desktop?'chatgpt':'none'),cliWorkdir:snapshot.cli_workdir||'',networkEnabled:snapshot.network_enabled!==false,connectivityEnabled:snapshot.connectivity_enabled!==false,autoCheckUpdates:!!snapshot.auto_check_updates,updateIntervalHours:[1,3,6,12,24].includes(snapshot.update_interval_hours)?snapshot.update_interval_hours:6};
  return Object.fromEntries(Object.entries(incoming).map(([key,value])=>[key,pending.has(key)?current[key]:value]));
}
