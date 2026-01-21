import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useAccountStore } from '../stores/useAccountStore';

export default function OAuthCallback() {
  const navigate = useNavigate();
  const { completeOAuthLogin } = useAccountStore();
  const [message, setMessage] = useState('Completing OAuth...');

  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const code = params.get('code');
    const error = params.get('error');
    const redirectUri = `${window.location.origin}/oauth/callback`;

    if (error) {
      setMessage(`OAuth failed: ${error}`);
      return;
    }
    if (!code) {
      setMessage('Missing OAuth code.');
      return;
    }

    completeOAuthLogin(code, redirectUri)
      .then(() => {
        setMessage('OAuth completed. Redirecting...');
        setTimeout(() => navigate('/accounts'), 1000);
      })
      .catch((err) => {
        setMessage(`OAuth failed: ${String(err)}`);
      });
  }, [completeOAuthLogin, navigate]);

  return (
    <div className="h-screen flex items-center justify-center bg-[#FAFBFC] dark:bg-base-300 text-gray-700 dark:text-gray-200">
      <div className="max-w-md text-center text-sm">{message}</div>
    </div>
  );
}
