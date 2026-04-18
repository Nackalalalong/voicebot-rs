'use client';

import {useEffect, useRef, useState} from 'react';

type ActiveCall = {
    session_id: string;
    phone_number: string;
    direction: string;
    campaign_id?: string;
    started_at: string;
    duration_secs: number;
};

export default function LiveMonitorPage() {
    const [calls, setCalls] = useState<ActiveCall[]>([]);
    const [connected, setConnected] = useState(false);
    const esRef = useRef<EventSource | null>(null);

    useEffect(() => {
        const es = new EventSource('/api/sse/sessions/live');
        esRef.current = es;

        es.onopen = () => setConnected(true);
        es.onerror = () => setConnected(false);

        es.addEventListener('active_calls', (e) => {
            try {
                const data = JSON.parse(e.data);
                setCalls(data.calls ?? []);
            } catch {}
        });

        return () => es.close();
    }, []);

    return (
        <div>
            <div className="flex items-center gap-4 mb-6">
                <h1 className="text-2xl font-semibold">Live Monitor</h1>
                <span
                    className={`flex items-center gap-1.5 text-xs ${connected ? 'text-green-600' : 'text-gray-400'}`}>
                    <span
                        className={`w-2 h-2 rounded-full ${connected ? 'bg-green-500 animate-pulse' : 'bg-gray-400'}`}
                    />
                    {connected ? 'Live' : 'Connecting…'}
                </span>
                <span className="ml-auto text-sm text-gray-500">
                    {calls.length} active call{calls.length !== 1 ? 's' : ''}
                </span>
            </div>

            <div className="bg-white rounded-lg shadow overflow-hidden">
                {calls.length === 0 ? (
                    <div className="px-4 py-12 text-center text-gray-400 text-sm">
                        No active calls right now.
                    </div>
                ) : (
                    <table className="w-full text-sm">
                        <thead className="bg-gray-50 text-gray-600 text-left text-xs">
                            <tr>
                                <th className="px-4 py-3">Session</th>
                                <th className="px-4 py-3">Phone</th>
                                <th className="px-4 py-3">Direction</th>
                                <th className="px-4 py-3">Campaign</th>
                                <th className="px-4 py-3">Duration</th>
                                <th className="px-4 py-3">Started</th>
                            </tr>
                        </thead>
                        <tbody className="divide-y">
                            {calls.map((c) => (
                                <tr key={c.session_id}>
                                    <td className="px-4 py-3 font-mono text-xs">
                                        {c.session_id.slice(0, 8)}…
                                    </td>
                                    <td className="px-4 py-3 font-mono">{c.phone_number}</td>
                                    <td className="px-4 py-3 capitalize">{c.direction}</td>
                                    <td className="px-4 py-3">{c.campaign_id ?? '—'}</td>
                                    <td className="px-4 py-3">
                                        <DurationCell startedAt={c.started_at} />
                                    </td>
                                    <td className="px-4 py-3">
                                        {new Date(c.started_at).toLocaleTimeString()}
                                    </td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                )}
            </div>
        </div>
    );
}

function DurationCell({startedAt}: {startedAt: string}) {
    const [secs, setSecs] = useState(() =>
        Math.floor((Date.now() - new Date(startedAt).getTime()) / 1000),
    );

    useEffect(() => {
        const interval = setInterval(() => {
            setSecs(Math.floor((Date.now() - new Date(startedAt).getTime()) / 1000));
        }, 1000);
        return () => clearInterval(interval);
    }, [startedAt]);

    const m = Math.floor(secs / 60);
    const s = secs % 60;
    return <span>{`${m}:${String(s).padStart(2, '0')}`}</span>;
}
