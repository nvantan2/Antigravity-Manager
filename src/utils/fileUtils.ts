export function downloadTextFile(filename: string, content: string) {
  const blob = new Blob([content], { type: 'application/json;charset=utf-8' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}

export async function pickTextFile(accept = '.json,.txt') {
  return new Promise<{ name: string; content: string } | null>((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = accept;
    input.onchange = () => {
      const file = input.files && input.files[0];
      if (!file) {
        resolve(null);
        return;
      }
      const reader = new FileReader();
      reader.onload = () => {
        resolve({ name: file.name, content: String(reader.result || '') });
      };
      reader.onerror = () => resolve(null);
      reader.readAsText(file);
    };
    input.click();
  });
}
