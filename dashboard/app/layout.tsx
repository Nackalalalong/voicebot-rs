import type {Metadata} from 'next';
import './globals.css';
import {QueryProvider} from '@/components/providers/query-provider';

export const metadata: Metadata = {
    title: 'Voicebot Platform',
    description: 'AI-powered telephony platform',
};

export default function RootLayout({children}: {children: React.ReactNode}) {
    return (
        <html lang="en">
            <body>
                <QueryProvider>{children}</QueryProvider>
            </body>
        </html>
    );
}
