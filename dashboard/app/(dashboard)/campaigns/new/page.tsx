'use client';

import {useMutation} from '@tanstack/react-query';
import {useRouter} from 'next/navigation';
import {useState} from 'react';
import Link from 'next/link';
import {apiFetch} from '@/lib/api';

const DEFAULT_PROMPT = `You are a helpful AI assistant. Be concise and friendly.`;

export default function NewCampaignPage() {
    const router = useRouter();
    const [form, setForm] = useState({
        name: '',
        language: 'en',
        voice_id: '',
        asr_model: 'whisper-1',
        llm_model: 'gpt-4o-mini',
        system_prompt: DEFAULT_PROMPT,
        direction: 'outbound',
        max_call_duration_secs: 300,
        recording_enabled: false,
    });
    const [error, setError] = useState('');

    const create = useMutation({
        mutationFn: () =>
            apiFetch('/api/v1/campaigns', {
                method: 'POST',
                body: JSON.stringify(form),
            }),
        onSuccess: (data) => router.push(`/campaigns/${data.id}`),
        onError: (e: any) => setError(e.message),
    });

    function set(field: string, value: unknown) {
        setForm((f) => ({...f, [field]: value}));
    }

    return (
        <div className="max-w-2xl">
            <div className="flex items-center gap-4 mb-6">
                <Link href="/campaigns" className="text-gray-400 hover:text-gray-600 text-sm">
                    ← Campaigns
                </Link>
                <h1 className="text-2xl font-semibold">New Campaign</h1>
            </div>

            <div className="bg-white rounded-lg shadow p-6 space-y-5">
                <div>
                    <label className="block text-sm font-medium mb-1">Name *</label>
                    <input
                        className="w-full border rounded px-3 py-2 text-sm"
                        value={form.name}
                        onChange={(e) => set('name', e.target.value)}
                        placeholder="My Campaign"
                    />
                </div>

                <div className="grid grid-cols-2 gap-4">
                    <div>
                        <label className="block text-sm font-medium mb-1">Direction</label>
                        <select
                            className="w-full border rounded px-3 py-2 text-sm"
                            value={form.direction}
                            onChange={(e) => set('direction', e.target.value)}>
                            <option value="outbound">Outbound</option>
                            <option value="inbound">Inbound</option>
                        </select>
                    </div>
                    <div>
                        <label className="block text-sm font-medium mb-1">Language</label>
                        <select
                            className="w-full border rounded px-3 py-2 text-sm"
                            value={form.language}
                            onChange={(e) => set('language', e.target.value)}>
                            <option value="en">English</option>
                            <option value="th">Thai</option>
                            <option value="zh">Chinese</option>
                            <option value="ja">Japanese</option>
                        </select>
                    </div>
                </div>

                <div className="grid grid-cols-2 gap-4">
                    <div>
                        <label className="block text-sm font-medium mb-1">LLM Model</label>
                        <input
                            className="w-full border rounded px-3 py-2 text-sm"
                            value={form.llm_model}
                            onChange={(e) => set('llm_model', e.target.value)}
                            placeholder="gpt-4o-mini"
                        />
                    </div>
                    <div>
                        <label className="block text-sm font-medium mb-1">ASR Model</label>
                        <input
                            className="w-full border rounded px-3 py-2 text-sm"
                            value={form.asr_model}
                            onChange={(e) => set('asr_model', e.target.value)}
                            placeholder="whisper-1"
                        />
                    </div>
                </div>

                <div className="grid grid-cols-2 gap-4">
                    <div>
                        <label className="block text-sm font-medium mb-1">Voice ID</label>
                        <input
                            className="w-full border rounded px-3 py-2 text-sm"
                            value={form.voice_id}
                            onChange={(e) => set('voice_id', e.target.value)}
                            placeholder="af_heart"
                        />
                    </div>
                    <div>
                        <label className="block text-sm font-medium mb-1">Max Duration (secs)</label>
                        <input
                            type="number"
                            className="w-full border rounded px-3 py-2 text-sm"
                            value={form.max_call_duration_secs}
                            onChange={(e) => set('max_call_duration_secs', Number(e.target.value))}
                        />
                    </div>
                </div>

                <div className="flex items-center gap-2">
                    <input
                        type="checkbox"
                        id="rec"
                        checked={form.recording_enabled}
                        onChange={(e) => set('recording_enabled', e.target.checked)}
                    />
                    <label htmlFor="rec" className="text-sm">Enable call recording</label>
                </div>

                <div>
                    <label className="block text-sm font-medium mb-1">System Prompt</label>
                    <textarea
                        rows={8}
                        className="w-full border rounded px-3 py-2 text-sm font-mono"
                        value={form.system_prompt}
                        onChange={(e) => set('system_prompt', e.target.value)}
                    />
                </div>

                {error && <p className="text-red-500 text-sm">{error}</p>}

                <div className="flex justify-end gap-3">
                    <Link
                        href="/campaigns"
                        className="px-4 py-2 border rounded text-sm hover:bg-gray-50">
                        Cancel
                    </Link>
                    <button
                        onClick={() => create.mutate()}
                        disabled={!form.name || create.isPending}
                        className="bg-blue-600 text-white px-4 py-2 rounded text-sm hover:bg-blue-700 disabled:opacity-50">
                        {create.isPending ? 'Creating…' : 'Create Campaign'}
                    </button>
                </div>
            </div>
        </div>
    );
}
