#region License Information (GPL v3)

/*
    ShareX - A program that allows you to take screenshots and share any file type
    Copyright (c) 2007-2026 ShareX Team

    This program is free software; you can redistribute it and/or
    modify it under the terms of the GNU General Public License
    as published by the Free Software Foundation; either version 2
    of the License, or (at your option) any later version.
*/

#endregion License Information (GPL v3)

using System;
using System.IO;
using System.Text;
using ShareX.HelpersLib;

namespace ShareX
{
    internal static class KaixinIntegrationResult
    {
        private static readonly UTF8Encoding Utf8NoBom = new UTF8Encoding(false);

        public static void Signal(TaskSettings taskSettings, string status)
        {
            Signal(taskSettings?.KaixinResultPath, status);
        }

        public static void Signal(string resultPath, string status)
        {
            if (!Program.KaixinIntegration || string.IsNullOrWhiteSpace(resultPath))
            {
                return;
            }

            string tempPath = null;
            try
            {
                string directory = Path.GetDirectoryName(resultPath);
                if (!string.IsNullOrEmpty(directory))
                {
                    Directory.CreateDirectory(directory);
                }

                tempPath = resultPath + ".tmp-" + Guid.NewGuid().ToString("N");
                File.WriteAllText(tempPath, status + "\n", Utf8NoBom);
                File.Move(tempPath, resultPath, true);
                DebugHelper.WriteLine($"Kaixin capture result: {status}");
            }
            catch (Exception e)
            {
                DebugHelper.WriteException(e, "Kaixin capture result handoff failed.");
            }
            finally
            {
                if (!string.IsNullOrEmpty(tempPath))
                {
                    try
                    {
                        File.Delete(tempPath);
                    }
                    catch
                    {
                    }
                }
            }
        }
    }
}
