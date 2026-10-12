import { useEffect, useState } from "react";

import { ModelStatus } from "./components/ModelStatus";
import { Sidebar } from "./components/Sidebar";
import { TopBar } from "./components/TopBar";
import type { Locale } from "./i18n";
import { useAppearance } from "./state/appearance";

export default function App() {
  const load = useAppearance((s) => s.load);
  const [locale] = useState<Locale>("en");

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="flex h-full flex-col bg-bg text-fg">
      <TopBar locale={locale} />
      <div className="flex min-h-0 flex-1">
        <Sidebar locale={locale} />
        <main className="flex min-w-0 flex-1 flex-col overflow-y-auto">
          <ModelStatus locale={locale} />
        </main>
      </div>
    </div>
  );
}
