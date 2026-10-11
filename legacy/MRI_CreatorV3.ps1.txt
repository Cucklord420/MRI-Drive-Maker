#----------VARIABLES----------
$IsDriveAvailable = $false
$RemovableDriveList = [System.Collections.ArrayList]::new()
$MRIFilePath = ""
$MRIPathIsCorrect = $false



#----------FUNCTIONS----------
function Format-Drives{
	foreach($Drive in $RemovableDriveList){
		Write-Host("Formatting drive: ", $Drive.DiskNumber," ",$Drive.FriendlyName,"`n")
		
		
			Write-Host("Clearing drive: ", $Drive.DiskNumber)
		#Need to set -confirm to $false for auto format
			Clear-Disk -Number $Drive.DiskNumber -RemoveData -confirm:$true
		

		$DiskInfo = Get-Disk -Number $Drive.DiskNumber
		if ($DiskInfo.size -gt 32000000000){
			Write-Host("Drive ", $Drive.DiskNumber, " is larger than 32 GB. Using Only 32GB of space for FAT32.")
		
		
		
		
			New-Partition $Drive.DiskNumber -AssignDriveLetter -IsActive -Size 32000000000
		}
		
		else{
			Write-Host("Drive ",$Drive.DiskNumber," smaller than or equal to 32GB. Using full drive space.")
			
		
			New-Partition $Drive.DiskNumber -AssignDriveLetter -IsActive -UseMaximumSize
		}
		
		$PartitionInfo = Get-Partition -DiskNumber $Drive.DiskNumber
		Write-Host("Formatting Drive",$PartitionInfo.DriveLetter, "to FAT32")
		
		Format-Volume -DriveLetter $PartitionInfo.DriveLetter -FileSystem FAT32 -NewFileSystemLabel "MRI" -Force
	}
}






function Create-MRI{
	foreach($Drive in $RemovableDriveList){
		$PartitionInfo = Get-Partition -DiskNumber $Drive.DiskNumber
		
		#Append \* for copy-item cmdlet to copy all items in folder
		$ItemsToCopy = $MRIFilePath+'\*'
		
		#Append :\ as $PartitionInfo.DriveLetter only lists a character, not a path
		$DestinationPath = $PartitionInfo.DriveLetter+':\'
		
		Write-Host("Writing MRI Files to drive: ", $PartitionInfo.DriveLetter," ", $Drive.FriendlyName,"`n")
		try{
			copy-item -path $ItemsToCopy -destination $DestinationPath -Recurse
		}
		catch{
			Write-Host("Could not copy files to drive ", $PartitionInfo.DriveLetter)
			Write-Host($_)
		}
	}
}





#----------MAIN----------

#Obtains a list of drives used for file storage.
$RemovableDriveList = Get-Disk | Where-Object -FilterScript {$_.Bustype -Eq "USB"}

#If there are no removable drives, notify the user.
if($RemovableDriveList.count -eq 0){
	
	Write-Host("There are no drives available for MRI Creation.")
}

#If there are removable drives detected, prompt user for MRI and Customizer locations, then list all available drives.
else{
	$IsDriveAvailable = $true
	
	#Ask User for file path to MRI files to copy
		while($MRIPathIsCorrect -ne $true){
	
		$MRIFilePath = Read-Host("What is the file path to MRI files? ")
	
		Write-Host("MRI file path is: ", $MRIFilePath,"`n")
	
		$MRIPathIsCorrect = Read-Host("Is the MRI file path correct? y/n")
			if(($MRIPathIsCorrect -eq "y") -or ($MRIPathIsCorrect -eq "yes")){
			$MRIPathIsCorrect = $true
			}
		}
	#Empty line for spacing
	Write-Host("")
	
	Write-Host("The following drives are available for MRI Creation:")
	
	#Print RemovableDriveList
	$RemovableDriveList
	}
	
	
#If drives are available to format, prompt user for confirmation that they want to format ALL drives
if ($IsDriveAvailable -eq $true){
		$DrivesToFormat = Read-Host("Would you like to convert all of these drives to MRI? y/n")
		
		if(($DrivesToFormat -eq "y") -or ($DrivesToFormat -eq "yes")){
			$UserConfirm = Read-Host("WARNING: THIS WILL WIPE ALL INFORMATION FROM THESE DRIVES. ARE YOU SURE? y/n")
		}
		
		if(($UserConfirm -eq "y") -or ($UserConfirm -eq "yes")){
			Write-Host("Formatting!`n")
			try{
				Format-Drives($RemovableDriveList) -ErrorAction exit
			}
			
			catch{
					Write-Host("Error: Could not format the disks.")
					Write-Host($_)
			}
			
			Create-MRI($RemovableDriveList)
			Write-host("MRI Creation complete.")
		}
		#If the user enter anything other than "y" or "yes", do not format drives.
		else{
			Write-Host("Aborting. Drives have not been formatted.")
		}
	}
	




