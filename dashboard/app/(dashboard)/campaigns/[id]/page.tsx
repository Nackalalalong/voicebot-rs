'use client';

import {useQuery, useMutation, useQueryClient} from '@tanstack/react-query';
import {useParams, useRouter} from 'next/navigation';
import {useState} from 'react';
import {apiFetch} from '@/lib/api';

export default function CampaignDetailPage() {
    const {id} = useParams<{id: string}>();
    const qc = useQueryClient();
    const router = useRouter();

    const {data: campaign, isLoading} = useQuery({
        queryKey: ['campaigns', id],
        queryFn: () => apiFetch(`/api/v1/campaigns/${id}`),
    });

    const [prompt, setPrompt] = useState<string | undefined>(undefined);

    const updatePrompt = useMutation({
        mutationFn: (system_prompt: string) =>
            apiFetch(`/api/v1/campaigns/${id}/prompt`, {
                method: 'PUT',
                body: JSON.stringify({system_prompt}),
            }),
        onSuccess: () => qc.invalidateQueries({queryKey: ['campaigns', id]}),
    });

    const updateStatus = useMutation({
        mutationFn: (status: string) =>
            apiFetch(`/api/v1/campaigns/${id}/status`, {
                method: 'PUT',
                body: JSON.stringify({status}),
            }),
        onSuccess: () => qc.invalidateQueries({queryKey: ['campaigns', id]}),
    });

    if (isLoading) return <p className="text-gray-500">Loading…</p>;
    if (!campaign) return <p className="text-red-500">Campaign not found</p>;

    return (
        <div className="max-w-3xl">
            <div className="flex items-center gap-4 mb-6">
                <h1 className="text-2xl font-semibold">{campaign.name}</h1>
                <span className="text-sm text-gray-500 capitalize">{campaign.status}</span>
            </div>

            <div className="bg-white rounded-lg shadow p-6 space-y-4">
                <div>
                    <label className="block text-sm font-medium mb-1">System Prompt</label>
                    <textarea
                        rows={8}
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

                <div className="flex gap-2 pt-2 border-t">
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
                </div>
            </div>
        </div>
    );
}
