const API_BASE_URL = import.meta.env.VITE_API_BASE_URL || 'http://127.0.0.1:8045';

interface InvokeResponse<T> {
  ok: boolean;
  data?: T;
  error?: string;
}

export async function request<T>(cmd: string, args?: any): Promise<T> {
  try {
    const resp = await fetch(`${API_BASE_URL}/api/invoke`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ cmd, args: args ?? {} }),
    });
    const payload = (await resp.json()) as InvokeResponse<T>;
    if (!resp.ok || !payload.ok) {
      const message = payload.error || `Request failed: ${resp.status}`;
      throw new Error(message);
    }
    return payload.data as T;
  } catch (error) {
    console.error(`API Error [${cmd}]:`, error);
    throw error;
  }
}
