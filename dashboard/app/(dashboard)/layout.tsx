import {Sidebar} from '@/components/layout/sidebar';

export default function DashboardLayout({children}: {children: React.ReactNode}) {
    return (
        <div className="flex h-screen overflow-hidden">
            <Sidebar />
            <main className="flex-1 overflow-auto p-6 bg-gray-50">{children}</main>
        </div>
    );
}
