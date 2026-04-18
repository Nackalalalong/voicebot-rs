'use client';

import {useQuery, useMutation, useQueryClient} from '@tanstack/react-query';
import {useState} from 'react';
import {apiFetch} from '@/lib/api';

type Tab = 'users' | 'usage' | 'providers';

export default function SettingsPage() {
    const [tab, setTab] = useState<Tab>('users');

    const TABS: {key: Tab; label: string}[] = [
        {key: 'users', label: 'Users'},
        {key: 'usage', label: 'Usage'},
        {key: 'providers', label: 'Providers'},
    ];

    return (
        <div className="max-w-4xl">
            <h1 className="text-2xl font-semibold mb-6">Settings</h1>

            <div className="flex gap-1 border-b mb-6">
                {TABS.map(({key, label}) => (
                    <button
                        key={key}
                        onClick={() => setTab(key)}
                        className={`px-4 py-2 text-sm border-b-2 -mb-px transition-colors ${
                            tab === key
                                ? 'border-blue-600 text-blue-600 font-medium'
                                : 'border-transparent text-gray-500 hover:text-gray-800'
                        }`}>
                        {label}
                    </button>
                ))}
            </div>

            {tab === 'users' && <UsersTab />}
            {tab === 'usage' && <UsageTab />}
            {tab === 'providers' && <ProvidersTab />}
        </div>
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Users Tab
// ──────────────────────────────────────────────────────────────────────────────
function UsersTab() {
    const qc = useQueryClient();
    const {data, isLoading} = useQuery({
        queryKey: ['users'],
        queryFn: () => apiFetch('/api/v1/users'),
    });
    const [inviteEmail, setInviteEmail] = useState('');
    const [inviteRole, setInviteRole] = useState('agent');
    const [inviteError, setInviteError] = useState('');

    const invite = useMutation({
        mutationFn: () =>
            apiFetch('/api/v1/users', {
                method: 'POST',
                body: JSON.stringify({email: inviteEmail, role: inviteRole}),
            }),
        onSuccess: () => {
            setInviteEmail('');
            setInviteError('');
            qc.invalidateQueries({queryKey: ['users']});
        },
        onError: (e: any) => setInviteError(e.message),
    });

    return (
        <div className="space-y-6">
            <div className="bg-white rounded-lg shadow p-5">
                <h2 className="text-sm font-semibold mb-4">Invite User</h2>
                <div className="flex gap-3">
                    <input
                        className="flex-1 border rounded px-3 py-2 text-sm"
                        placeholder="email@example.com"
                        value={inviteEmail}
                        onChange={(e) => setInviteEmail(e.target.value)}
                    />
                    <select
                        className="border rounded px-3 py-2 text-sm"
                        value={inviteRole}
                        onChange={(e) => setInviteRole(e.target.value)}>
                        <option value="admin">Admin</option>
                        <option value="agent">Agent</option>
                        <option value="viewer">Viewer</option>
                    </select>
                    <button
                        onClick={() => invite.mutate()}
                        disabled={!inviteEmail || invite.isPending}
                        className="bg-blue-600 text-white px-4 py-2 rounded text-sm hover:bg-blue-700 disabled:opacity-50">
                        Invite
                    </button>
                </div>
                {inviteError && <p className="text-red-500 text-sm mt-2">{inviteError}</p>}
            </div>

            {isLoading ? (
                <p className="text-gray-500 text-sm">Loading…</p>
            ) : (
                <div className="bg-white rounded-lg shadow overflow-hidden">
                    <table className="w-full text-sm">
                        <thead className="bg-gray-50 text-gray-600 text-left text-xs">
                            <tr>
                                <th className="px-4 py-3">Email</th>
                                <th className="px-4 py-3">Name</th>
                                <th className="px-4 py-3">Role</th>
                                <th className="px-4 py-3">Status</th>
                                <th className="px-4 py-3">Joined</th>
                            </tr>
                        </thead>
                        <tbody className="divide-y">
                            {data?.items?.map((u: any) => (
                                <tr key={u.id}>
                                    <td className="px-4 py-3">{u.email}</td>
                                    <td className="px-4 py-3">{u.display_name ?? '—'}</td>
                                    <td className="px-4 py-3 capitalize">{u.role}</td>
                                    <td className="px-4 py-3 capitalize">{u.status}</td>
                                    <td className="px-4 py-3">
                                        {new Date(u.created_at).toLocaleDateString()}
                                    </td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
            )}
        </div>
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Usage Tab
// ──────────────────────────────────────────────────────────────────────────────
function UsageTab() {
    const [days, setDays] = useState(30);

    const {data, isLoading} = useQuery({
        queryKey: ['usage', days],
        queryFn: () => apiFetch(`/api/v1/usage?days=${days}`),
    });

    return (
        <div className="space-y-4">
            <div className="flex items-center gap-3">
                <label className="text-sm text-gray-600">Period:</label>
                {[7, 30, 90].map((d) => (
                    <button
                        key={d}
                        onClick={() => setDays(d)}
                        className={`px-3 py-1 rounded text-sm border transition-colors ${
                            days === d
                                ? 'bg-blue-600 text-white border-blue-600'
                                : 'border-gray-300 text-gray-600 hover:bg-gray-50'
                        }`}>
                        {d}d
                    </button>
                ))}
            </div>

            {isLoading ? (
                <p className="text-gray-500 text-sm">Loading…</p>
            ) : (
                <div className="grid grid-cols-2 md:grid-cols-3 gap-4">
                    <UsageCard label="Total Calls" value={data?.total_calls ?? 0} />
                    <UsageCard label="Total Minutes" value={data?.total_minutes != null ? `${Math.round(data.total_minutes)}m` : '—'} />
                    <UsageCard label="Unique Callers" value={data?.unique_callers ?? 0} />
                    <UsageCard label="Completed Calls" value={data?.completed_calls ?? 0} />
                    <UsageCard label="Failed Calls" value={data?.failed_calls ?? 0} />
                    <UsageCard label="Avg Duration" value={data?.avg_duration_secs ? `${Math.round(data.avg_duration_secs)}s` : '—'} />
                </div>
            )}
        </div>
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Providers Tab
// ──────────────────────────────────────────────────────────────────────────────
function ProvidersTab() {
    return (
        <div className="bg-white rounded-lg shadow p-5 space-y-4">
            <h2 className="text-sm font-semibold">Provider Configuration</h2>
            <p className="text-sm text-gray-500">
                Provider settings (LLM endpoint, ASR endpoint, TTS endpoint) are configured via
                environment variables or <code className="bg-gray-100 px-1 rounded">config.toml</code>.
                Refer to the deployment documentation for details.
            </p>
            <div className="grid grid-cols-1 gap-3 text-sm">
                {[
                    {label: 'LLM', env: 'LLM_BASE_URL'},
                    {label: 'ASR (Speaches)', env: 'SPEACHES_URL'},
                    {label: 'TTS (Speaches)', env: 'SPEACHES_URL'},
                    {label: 'Storage (S3)', env: 'S3_ENDPOINT_URL'},
                    {label: 'Asterisk ARI', env: 'ARI_BASE_URL'},
                ].map(({label, env}) => (
                    <div key={env} className="flex items-center gap-3 p-3 bg-gray-50 rounded">
                        <span className="w-32 font-medium text-gray-700">{label}</span>
                        <code className="text-xs text-gray-500">{env}</code>
                    </div>
                ))}
            </div>
        </div>
    );
}

function UsageCard({label, value}: {label: string; value: string | number}) {
    return (
        <div className="bg-white rounded-lg shadow p-4">
            <p className="text-xs text-gray-500">{label}</p>
            <p className="text-2xl font-bold mt-1">{value}</p>
        </div>
    );
}
