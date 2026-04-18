'use client';

import {useQuery} from '@tanstack/react-query';
import {useEffect, useRef, useState} from 'react';
import {apiFetch} from '@/lib/api';

export default function OverviewPage() {
    const {data: campaigns} = useQuery({
        queryKey: ['campaigns'],
        queryFn: () => apiFetch('/api/v1/campaigns'),
    });
    const {data: usage} = useQuery({
        queryKey: ['usage'],
        queryFn: () => apiFetch('/api/v1/usage?days=1'),
    });
    const [activeCalls, setActiveCalls] = useState<number | null>(null);
    const esRef = useRef<EventSource | null>(null);

    useEffect(() => {
        const es = new EventSource('/api/sse/metrics/live');
        esRef.current = es;
        es.addEventListener('active_calls', (e) => {
            try {
                const d = JSON.parse(e.data);
                setActiveCalls(d.count ?? 0);
            } catch {}
        });
        es.onerror = () => setActiveCalls(0);
        return () => es.close();
    }, []);

    const activeCampaigns =
        campaigns?.items?.filter((c: any) => c.status === 'active').length ?? '—';

    return (
        <div>
            <h1 className="text-2xl font-semibold mb-6">Overview</h1>
            <div className="grid grid-cols-1 md:grid-cols-4 gap-4">
                <StatCard label="Total Campaigns" value={campaigns?.total ?? '—'} />
                <StatCard label="Active Campaigns" value={activeCampaigns} />
                <StatCard
                    label="Active Calls"
                    value={activeCalls !== null ? activeCalls : '—'}
                    live
                />
                <StatCard label="Calls Today" value={usage?.total_calls ?? '—'} />
            </div>

            {campaigns?.items && campaigns.items.length > 0 && (
                <div className="mt-8">
                    <h2 className="text-lg font-semibold mb-4">Recent Campaigns</h2>
                    <div className="bg-white rounded-lg shadow overflow-hidden">
                        <table className="w-full text-sm">
                            <thead className="bg-gray-50 text-gray-600 text-left">
                                <tr>
                                    <th className="px-4 py-3">Name</th>
                                    <th className="px-4 py-3">Status</th>
                                    <th className="px-4 py-3">Language</th>
                                    <th className="px-4 py-3">Created</th>
                                </tr>
                            </thead>
                            <tbody className="divide-y">
                                {campaigns.items.slice(0, 5).map((c: any) => (
                                    <tr key={c.id}>
                                        <td className="px-4 py-3 font-medium">{c.name}</td>
                                        <td className="px-4 py-3 capitalize">{c.status}</td>
                                        <td className="px-4 py-3">{c.language}</td>
                                        <td className="px-4 py-3">
                                            {new Date(c.created_at).toLocaleDateString()}
                                        </td>
                                    </tr>
                                ))}
                            </tbody>
                        </table>
                    </div>
                </div>
            )}
        </div>
    );
}

function StatCard({label, value, live}: {label: string; value: string | number; live?: boolean}) {
    return (
        <div className="bg-white rounded-lg shadow p-5">
            <div className="flex items-center justify-between mb-1">
                <p className="text-sm text-gray-500">{label}</p>
                {live && (
                    <span className="flex items-center gap-1 text-xs text-green-600">
                        <span className="w-2 h-2 rounded-full bg-green-500 animate-pulse" />
                        Live
                    </span>
                )}
            </div>
            <p className="text-3xl font-bold">{value}</p>
        </div>
    );
}
