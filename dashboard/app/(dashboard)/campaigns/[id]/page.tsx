'use client';

import {useQuery, useMutation, useQueryClient} from '@tanstack/react-query';
import {useParams, useRouter} from 'next/navigation';
import {useState, useRef} from 'react';
import Link from 'next/link';
import {apiFetch} from '@/lib/api';

type Tab = 'config' | 'contacts' | 'calls' | 'analytics' | 'metrics';

export default function CampaignDetailPage() {
    const {id} = useParams<{id: string}>();
    const qc = useQueryClient();
    const router = useRouter();
    const [tab, setTab] = useState<Tab>('config');

    const {data: campaign, isLoading} = useQuery({
        queryKey: ['campaigns', id],
        queryFn: () => apiFetch(`/api/v1/campaigns/${id}`),
    });

    const updateStatus = useMutation({
        mutationFn: (status: string) =>
            apiFetch(`/api/v1/campaigns/${id}/status`, {
                method: 'PUT',
                body: JSON.stringify({status}),
            }),
        onSuccess: () => qc.invalidateQueries({queryKey: ['campaigns', id]}),
    });

    const deleteCampaign = useMutation({
        mutationFn: () => apiFetch(`/api/v1/campaigns/${id}`, {method: 'DELETE'}),
        onSuccess: () => router.push('/campaigns'),
    });

    if (isLoading) return <p className="text-gray-500">Loading…</p>;
    if (!campaign) return <p className="text-red-500">Campaign not found</p>;

    const TABS: {key: Tab; label: string}[] = [
        {key: 'config', label: 'Config'},
        {key: 'contacts', label: 'Contacts'},
        {key: 'calls', label: 'Calls'},
        {key: 'analytics', label: 'Analytics'},
        {key: 'metrics', label: 'Metrics'},
    ];

    return (
        <div className="max-w-5xl">
            <div className="flex items-center gap-4 mb-2">
                <Link href="/campaigns" className="text-gray-400 hover:text-gray-600 text-sm">
                    ← Campaigns
                </Link>
            </div>
            <div className="flex items-center gap-4 mb-6">
                <h1 className="text-2xl font-semibold">{campaign.name}</h1>
                <StatusBadge status={campaign.status} />
                <div className="ml-auto flex gap-2">
                    {campaign.status !== 'active' && (
                        <button
                            onClick={() => updateStatus.mutate('active')}
                            className="bg-green-600 text-white px-3 py-1.5 rounded text-sm hover:bg-green-700">
                            Activate
                        </button>
                    )}
                    {campaign.status === 'active' && (
                        <button
                            onClick={() => updateStatus.mutate('paused')}
                            className="bg-yellow-500 text-white px-3 py-1.5 rounded text-sm hover:bg-yellow-600">
                            Pause
                        </button>
                    )}
                    <button
                        onClick={() => {
                            if (confirm('Delete this campaign?')) deleteCampaign.mutate();
                        }}
                        className="border border-red-300 text-red-600 px-3 py-1.5 rounded text-sm hover:bg-red-50">
                        Delete
                    </button>
                </div>
            </div>

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

            {tab === 'config' && <ConfigTab campaign={campaign} id={id} qc={qc} />}
            {tab === 'contacts' && <ContactsTab id={id} qc={qc} />}
            {tab === 'calls' && <CallsTab id={id} />}
            {tab === 'analytics' && <AnalyticsTab id={id} />}
            {tab === 'metrics' && <MetricsTab campaign={campaign} id={id} qc={qc} />}
        </div>
    );
}

function ConfigTab({campaign, id, qc}: {campaign: any; id: string; qc: any}) {
    const [prompt, setPrompt] = useState<string | undefined>(undefined);

    const updatePrompt = useMutation({
        mutationFn: (system_prompt: string) =>
            apiFetch(`/api/v1/campaigns/${id}/prompt`, {
                method: 'PUT',
                body: JSON.stringify({system_prompt}),
            }),
        onSuccess: () => qc.invalidateQueries({queryKey: ['campaigns', id]}),
    });

    return (
        <div className="bg-white rounded-lg shadow p-6 space-y-6">
            <div className="grid grid-cols-2 gap-4 text-sm">
                <Info label="Language" value={campaign.language ?? '—'} />
                <Info label="Voice" value={campaign.voice_id ?? '—'} />
                <Info label="ASR Model" value={campaign.asr_model ?? '—'} />
                <Info label="LLM Model" value={campaign.llm_model ?? '—'} />
                <Info label="Recording" value={campaign.recording_enabled ? 'Enabled' : 'Disabled'} />
                <Info label="Max Duration" value={campaign.max_call_duration_secs ? `${campaign.max_call_duration_secs}s` : '—'} />
            </div>
            <div>
                <label className="block text-sm font-medium mb-1">System Prompt</label>
                <textarea
                    rows={10}
                    className="w-full border rounded px-3 py-2 text-sm font-mono"
                    defaultValue={campaign.system_prompt}
                    onChange={(e) => setPrompt(e.target.value)}
                />
                <button
                    onClick={() => prompt && updatePrompt.mutate(prompt)}
                    disabled={!prompt || updatePrompt.isPending}
                    className="mt-2 bg-blue-600 text-white px-4 py-1.5 rounded text-sm hover:bg-blue-700 disabled:opacity-50">
                    {updatePrompt.isPending ? 'Saving…' : 'Save Prompt'}
                </button>
            </div>
        </div>
    );
}

function ContactsTab({id, qc}: {id: string; qc: any}) {
    const fileRef = useRef<HTMLInputElement>(null);
    const [uploading, setUploading] = useState(false);
    const [uploadError, setUploadError] = useState('');

    const {data, isLoading} = useQuery({
        queryKey: ['contacts', id],
        queryFn: () => apiFetch(`/api/v1/campaigns/${id}/contacts`),
    });

    async function handleCsvUpload() {
        const file = fileRef.current?.files?.[0];
        if (!file) return;
        setUploading(true);
        setUploadError('');
        try {
            const text = await file.text();
            await apiFetch(`/api/v1/campaigns/${id}/contacts/import`, {
                method: 'POST',
                body: JSON.stringify({csv: text}),
            });
            qc.invalidateQueries({queryKey: ['contacts', id]});
        } catch (e: any) {
            setUploadError(e.message);
        } finally {
            setUploading(false);
            if (fileRef.current) fileRef.current.value = '';
        }
    }

    return (
        <div className="space-y-4">
            <div className="bg-white rounded-lg shadow p-4 flex items-center gap-4 flex-wrap">
                <div>
                    <p className="text-sm font-medium">Import contacts from CSV</p>
                    <p className="text-xs text-gray-500 mt-0.5">Required: phone_number. Optional: name, metadata</p>
                </div>
                <div className="ml-auto flex items-center gap-3">
                    <input ref={fileRef} type="file" accept=".csv" className="text-sm" />
                    <button
                        onClick={handleCsvUpload}
                        disabled={uploading}
                        className="bg-blue-600 text-white px-3 py-1.5 rounded text-sm hover:bg-blue-700 disabled:opacity-50">
                        {uploading ? 'Uploading…' : 'Import'}
                    </button>
                </div>
                {uploadError && <p className="text-red-500 text-sm w-full">{uploadError}</p>}
            </div>
            {isLoading ? (
                <p className="text-gray-500 text-sm">Loading…</p>
            ) : (
                <div className="bg-white rounded-lg shadow overflow-hidden">
                    <div className="px-4 py-3 bg-gray-50 text-xs text-gray-500 font-medium">
                        {data?.total ?? 0} contacts
                    </div>
                    <table className="w-full text-sm">
                        <thead className="bg-gray-50 text-gray-600 text-left text-xs">
                            <tr>
                                <th className="px-4 py-2">Phone</th>
                                <th className="px-4 py-2">Name</th>
                                <th className="px-4 py-2">Status</th>
                                <th className="px-4 py-2">Attempts</th>
                            </tr>
                        </thead>
                        <tbody className="divide-y">
                            {data?.items?.map((c: any) => (
                                <tr key={c.id}>
                                    <td className="px-4 py-2 font-mono">{c.phone_number}</td>
                                    <td className="px-4 py-2">{c.name ?? '—'}</td>
                                    <td className="px-4 py-2 capitalize">{c.status}</td>
                                    <td className="px-4 py-2">{c.attempt_count ?? 0}</td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
            )}
        </div>
    );
}

function CallsTab({id}: {id: string}) {
    const {data, isLoading} = useQuery({
        queryKey: ['campaign-calls', id],
        queryFn: () => apiFetch(`/api/v1/campaigns/${id}/calls`),
    });

    return (
        <div>
            {isLoading ? (
                <p className="text-gray-500 text-sm">Loading…</p>
            ) : (
                <div className="bg-white rounded-lg shadow overflow-hidden">
                    <table className="w-full text-sm">
                        <thead className="bg-gray-50 text-gray-600 text-left text-xs">
                            <tr>
                                <th className="px-4 py-3">Phone</th>
                                <th className="px-4 py-3">Direction</th>
                                <th className="px-4 py-3">Status</th>
                                <th className="px-4 py-3">Duration</th>
                                <th className="px-4 py-3">Sentiment</th>
                                <th className="px-4 py-3">Date</th>
                                <th className="px-4 py-3" />
                            </tr>
                        </thead>
                        <tbody className="divide-y">
                            {data?.items?.map((c: any) => (
                                <tr key={c.id} className="hover:bg-gray-50">
                                    <td className="px-4 py-3 font-mono">{c.phone_number}</td>
                                    <td className="px-4 py-3 capitalize">{c.direction}</td>
                                    <td className="px-4 py-3 capitalize">{c.status}</td>
                                    <td className="px-4 py-3">{c.duration_secs != null ? `${c.duration_secs}s` : '—'}</td>
                                    <td className="px-4 py-3 capitalize">{c.sentiment ?? '—'}</td>
                                    <td className="px-4 py-3">{new Date(c.created_at).toLocaleString()}</td>
                                    <td className="px-4 py-3">
                                        <Link href={`/calls/${c.id}`} className="text-blue-600 text-xs hover:underline">
                                            View
                                        </Link>
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

function AnalyticsTab({id}: {id: string}) {
    const {data, isLoading} = useQuery({
        queryKey: ['campaign-analytics', id],
        queryFn: () => apiFetch(`/api/v1/campaigns/${id}/analytics`),
    });

    if (isLoading) return <p className="text-gray-500 text-sm">Loading…</p>;
    if (!data) return <p className="text-gray-400 text-sm">No analytics yet.</p>;

    const sentimentCounts = (data.sentiment_breakdown ?? []) as {sentiment: string; count: number}[];

    return (
        <div className="space-y-6">
            <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
                <AnalyticCard label="Total Calls" value={data.total_calls ?? 0} />
                <AnalyticCard label="Completed" value={data.completed_calls ?? 0} />
                <AnalyticCard label="Avg Duration" value={data.avg_duration_secs ? `${Math.round(data.avg_duration_secs)}s` : '—'} />
                <AnalyticCard label="Answer Rate" value={data.answer_rate ? `${Math.round(data.answer_rate * 100)}%` : '—'} />
            </div>
            {sentimentCounts.length > 0 && (
                <div className="bg-white rounded-lg shadow p-5">
                    <h3 className="text-sm font-semibold mb-3">Sentiment Breakdown</h3>
                    <div className="space-y-2">
                        {sentimentCounts.map(({sentiment, count}) => {
                            const total = sentimentCounts.reduce((s, x) => s + x.count, 0);
                            const pct = total > 0 ? Math.round((count / total) * 100) : 0;
                            return (
                                <div key={sentiment} className="flex items-center gap-3">
                                    <span className="w-20 text-xs capitalize text-gray-600">{sentiment}</span>
                                    <div className="flex-1 bg-gray-100 rounded h-4 overflow-hidden">
                                        <div
                                            className={`h-full rounded ${sentiment === 'positive' ? 'bg-green-500' : sentiment === 'negative' ? 'bg-red-400' : 'bg-yellow-400'}`}
                                            style={{width: `${pct}%`}}
                                        />
                                    </div>
                                    <span className="text-xs text-gray-500 w-16 text-right">{count} ({pct}%)</span>
                                </div>
                            );
                        })}
                    </div>
                </div>
            )}
        </div>
    );
}

function MetricsTab({campaign, id, qc}: {campaign: any; id: string; qc: any}) {
    const [json, setJson] = useState(() =>
        JSON.stringify(campaign.custom_metrics_config ?? {metrics: []}, null, 2),
    );
    const [error, setError] = useState('');

    const save = useMutation({
        mutationFn: (config: unknown) =>
            apiFetch(`/api/v1/campaigns/${id}/metrics`, {
                method: 'PUT',
                body: JSON.stringify({custom_metrics_config: config}),
            }),
        onSuccess: () => qc.invalidateQueries({queryKey: ['campaigns', id]}),
    });

    function handleSave() {
        setError('');
        let parsed: unknown;
        try {
            parsed = JSON.parse(json);
        } catch {
            setError('Invalid JSON');
            return;
        }
        save.mutate(parsed);
    }

    return (
        <div className="bg-white rounded-lg shadow p-6 space-y-4">
            <div>
                <p className="text-sm font-medium mb-1">Custom Metrics Config (JSON)</p>
                <p className="text-xs text-gray-500 mb-3">
                    Define metrics extracted per call by the post-call LLM analysis.
                </p>
                <textarea
                    rows={12}
                    className="w-full border rounded px-3 py-2 text-sm font-mono"
                    value={json}
                    onChange={(e) => setJson(e.target.value)}
                />
                {error && <p className="text-red-500 text-xs mt-1">{error}</p>}
            </div>
            <button
                onClick={handleSave}
                disabled={save.isPending}
                className="bg-blue-600 text-white px-4 py-1.5 rounded text-sm hover:bg-blue-700 disabled:opacity-50">
                {save.isPending ? 'Saving…' : 'Save Metrics Config'}
            </button>
        </div>
    );
}

function StatusBadge({status}: {status: string}) {
    const colors: Record<string, string> = {
        active: 'bg-green-100 text-green-700',
        paused: 'bg-yellow-100 text-yellow-700',
        completed: 'bg-gray-100 text-gray-600',
        draft: 'bg-blue-100 text-blue-600',
    };
    return (
        <span className={`text-xs px-2 py-0.5 rounded-full capitalize ${colors[status] ?? 'bg-gray-100 text-gray-600'}`}>
            {status}
        </span>
    );
}

function Info({label, value}: {label: string; value: string}) {
    return (
        <div>
            <p className="text-xs text-gray-500">{label}</p>
            <p className="font-medium">{value}</p>
        </div>
    );
}

function AnalyticCard({label, value}: {label: string; value: string | number}) {
    return (
        <div className="bg-white rounded-lg shadow p-4">
            <p className="text-xs text-gray-500">{label}</p>
            <p className="text-2xl font-bold mt-1">{value}</p>
        </div>
    );
}
