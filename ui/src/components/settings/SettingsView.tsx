import { useMemo } from 'react';
import { useGetSettingsQuery } from '@/store/apiSlice';
import { mapSettingsRows, type Setting } from '@/services/systemTableService';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Loader2, RefreshCw, Settings as SettingsIcon } from 'lucide-react';

interface SettingsViewProps {
  filterCategory?: string;
}

export function SettingsView({ filterCategory }: SettingsViewProps) {
  const {
    data = [],
    isFetching: isLoading,
    error,
    refetch,
  } = useGetSettingsQuery();

  const settings = useMemo(() => mapSettingsRows(data), [data]);
  const groupedSettings = useMemo(() => {
    const groups: Record<string, Setting[]> = {};
    settings.forEach((setting) => {
      if (!groups[setting.category]) {
        groups[setting.category] = [];
      }
      groups[setting.category].push(setting);
    });
    return groups;
  }, [settings]);

  if (error) {
    return (
      <Card className="border-red-200">
        <CardContent className="py-6">
          <p className="text-red-700">{"error" in error ? error.error : "Failed to fetch settings"}</p>
          <Button variant="outline" onClick={() => void refetch()} className="mt-2">
            Retry
          </Button>
        </CardContent>
      </Card>
    );
  }

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-8">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    );
  }

  // Filter categories if specified
  const categories = filterCategory 
    ? Object.keys(groupedSettings).filter(cat => 
        cat.toLowerCase().includes(filterCategory.toLowerCase())
      )
    : Object.keys(groupedSettings);

  if (categories.length === 0) {
    return (
      <div className="text-muted-foreground text-sm py-4">
        No settings available for this category.
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Toolbar - only show for full view */}
      {!filterCategory && (
      <div className="flex items-center justify-end">
        <Button variant="outline" size="icon" onClick={() => void refetch()} disabled={isLoading}>
          <RefreshCw className={`h-4 w-4 ${isLoading ? 'animate-spin' : ''}`} />
        </Button>
      </div>
      )}

      {/* Settings by Category */}
      {categories.map((category) => (
        <Card key={category}>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <SettingsIcon className="h-5 w-5" />
              {category}
            </CardTitle>
          </CardHeader>
          <CardContent>
            <div className="divide-y">
              {groupedSettings[category].map((setting: Setting) => (
                <div key={setting.name} className="py-4 first:pt-0 last:pb-0">
                  <div className="flex flex-col items-start justify-between gap-3 sm:flex-row sm:gap-4">
                    <div className="min-w-0">
                      <h4 className="font-medium">{setting.name}</h4>
                      <p className="text-sm text-muted-foreground mt-1">
                        {setting.description}
                      </p>
                    </div>
                    <div className="max-w-full text-left sm:max-w-[45%] sm:text-right">
                      <code className="inline-block max-w-full break-all rounded bg-muted px-2 py-1 text-sm">
                        {setting.value}
                      </code>
                    </div>
                  </div>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
      ))}
    </div>
  );
}
