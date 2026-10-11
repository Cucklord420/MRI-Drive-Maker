// SPDX-License-Identifier: GPL-3.0-only
// Added 10-10-2026. Windows process-tree containment and volume flushing.
using System;
using System.Text;
using System.Runtime.InteropServices;
using System.ComponentModel;

namespace MRIBatch {
    public sealed class NativeProcess : IDisposable {
        private IntPtr job, process;
        private bool disposed;
        [StructLayout(LayoutKind.Sequential)] private struct SecurityAttributes {
            public int Length; public IntPtr Descriptor;
            [MarshalAs(UnmanagedType.Bool)] public bool Inherit;
        }
        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)] private struct StartupInfo {
            public int Size; public string Reserved, Desktop, Title;
            public int X, Y, XSize, YSize, XChars, YChars, Fill, Flags;
            public short Show, ReservedSize; public IntPtr Reserved2, Input, Output, Error;
        }
        [StructLayout(LayoutKind.Sequential)] private struct ProcessInfo {
            public IntPtr Process, Thread; public uint ProcessId, ThreadId;
        }
        [StructLayout(LayoutKind.Sequential)] private struct BasicLimits {
            public long PerProcessTime, PerJobTime; public uint Flags;
            public UIntPtr MinWorkingSet, MaxWorkingSet; public uint ActiveLimit;
            public UIntPtr Affinity; public uint Priority, Scheduling;
        }
        [StructLayout(LayoutKind.Sequential)] private struct IOInfo {
            public ulong ReadOps, WriteOps, OtherOps, ReadBytes, WriteBytes, OtherBytes;
        }
        [StructLayout(LayoutKind.Sequential)] private struct ExtendedLimits {
            public BasicLimits Basic; public IOInfo IO;
            public UIntPtr ProcessMemory, JobMemory, PeakProcessMemory, PeakJobMemory;
        }
        [StructLayout(LayoutKind.Sequential)] private struct Accounting {
            public long User, Kernel, PeriodUser, PeriodKernel;
            public uint PageFaults, TotalProcesses, ActiveProcesses, TerminatedProcesses;
        }
        [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern IntPtr CreateJobObject(IntPtr security, string name);
        [DllImport("kernel32.dll", SetLastError=true)] private static extern bool SetInformationJobObject(IntPtr handle, int infoClass, ref ExtendedLimits info, int size);
        [DllImport("kernel32.dll", SetLastError=true)] private static extern bool QueryInformationJobObject(IntPtr handle, int infoClass, out Accounting info, int size, IntPtr returned);
        [DllImport("kernel32.dll", SetLastError=true)] private static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
        [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern bool CreateProcess(string application, StringBuilder commandLine, IntPtr procSecurity, IntPtr threadSecurity, bool inherit, uint flags, IntPtr environment, string directory, ref StartupInfo startup, out ProcessInfo info);
        [DllImport("kernel32.dll", SetLastError=true)] private static extern uint ResumeThread(IntPtr thread);
        [DllImport("kernel32.dll", SetLastError=true)] private static extern bool GetExitCodeProcess(IntPtr process, out uint exit);
        [DllImport("kernel32.dll", SetLastError=true)] private static extern bool TerminateProcess(IntPtr process, uint code);
        [DllImport("kernel32.dll", SetLastError=true)] private static extern bool CloseHandle(IntPtr handle);
        [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern IntPtr CreateFile(string path, uint access, uint share, ref SecurityAttributes security, uint disposition, uint attributes, IntPtr template);
        [DllImport("kernel32.dll", SetLastError=true)] private static extern bool FlushFileBuffers(IntPtr handle);
        private static readonly IntPtr Invalid = new IntPtr(-1);
        private static void Check(bool value) { if (!value) throw new Win32Exception(Marshal.GetLastWin32Error()); }
        private static IntPtr Open(string path, uint access, uint disposition, bool inherit) {
            SecurityAttributes sa = new SecurityAttributes { Length=Marshal.SizeOf(typeof(SecurityAttributes)), Inherit=inherit };
            IntPtr h=CreateFile(path,access,3,ref sa,disposition,0x80,IntPtr.Zero);
            if(h==Invalid) throw new Win32Exception(Marshal.GetLastWin32Error());
            return h;
        }
        public static string Quote(string text) {
            if(text.IndexOf('\0')>=0) throw new ArgumentException("NUL in argument");
            StringBuilder b=new StringBuilder("\""); int slashes=0;
            foreach(char c in text) {
                if(c=='\\') { slashes++; continue; }
                if(c=='\"') { b.Append('\\',slashes*2+1); b.Append(c); }
                else { b.Append('\\',slashes); b.Append(c); }
                slashes=0;
            }
            b.Append('\\',slashes*2); b.Append('"'); return b.ToString();
        }
        public NativeProcess(string application, string arguments, string directory, string logPath, bool interactive) {
            IntPtr output=IntPtr.Zero,input=IntPtr.Zero,thread=IntPtr.Zero;
            try {
                job=CreateJobObject(IntPtr.Zero,null);
                if(job==IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
                ExtendedLimits limits=new ExtendedLimits(); limits.Basic.Flags=0x2000; // KILL_ON_JOB_CLOSE; no breakaway
                Check(SetInformationJobObject(job,9,ref limits,Marshal.SizeOf(typeof(ExtendedLimits))));
                StartupInfo si=new StartupInfo { Size=Marshal.SizeOf(typeof(StartupInfo)) };
                if(!interactive) {
                    output=Open(logPath,0x40000000,2,true); input=Open("NUL",0x80000000,3,true);
                    si.Flags=0x100; si.Output=output; si.Error=output; si.Input=input;
                }
                ProcessInfo pi;
                Check(CreateProcess(application,new StringBuilder(Quote(application)+" "+arguments),IntPtr.Zero,IntPtr.Zero,!interactive,
                    0x4u | (interactive ? 0x10u : 0x08000000u),IntPtr.Zero,directory,ref si,out pi));
                process=pi.Process; thread=pi.Thread;
                // Assign before resume: even immediately detached children inherit containment.
                Check(AssignProcessToJobObject(job,process));
                if(ResumeThread(thread)==uint.MaxValue) throw new Win32Exception(Marshal.GetLastWin32Error());
            } catch {
                if(process!=IntPtr.Zero) TerminateProcess(process,1);
                Dispose(); throw;
            } finally {
                if(thread!=IntPtr.Zero) CloseHandle(thread);
                if(output!=IntPtr.Zero) CloseHandle(output);
                if(input!=IntPtr.Zero) CloseHandle(input);
            }
        }
        public uint ActiveProcesses {
            get { Accounting a; Check(QueryInformationJobObject(job,1,out a,Marshal.SizeOf(typeof(Accounting)),IntPtr.Zero)); return a.ActiveProcesses; }
        }
        public uint ExitCode {
            get { uint code; Check(GetExitCodeProcess(process,out code)); return code; }
        }
        public static void FlushVolume(string volumePath) {
            IntPtr h=Open(volumePath.TrimEnd('\\'),0x40000000,3,false);
            try { Check(FlushFileBuffers(h)); } finally { CloseHandle(h); }
        }
        public void Dispose() {
            if(disposed) return; disposed=true;
            if(job!=IntPtr.Zero) { CloseHandle(job); job=IntPtr.Zero; }
            if(process!=IntPtr.Zero) { CloseHandle(process); process=IntPtr.Zero; }
        }
    }
}
