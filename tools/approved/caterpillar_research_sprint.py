import json

def summarize_funding(fund_string):
    data = {
        "grant_number": "",
        "funding_agency": "",
        "project_title": ""
    }
    
    # Parse the string for grant number, agency, and title
    fund_parts = fund_string.split()
    
    if "NRF" in fund_parts:
        index = fund_parts.index("NRF")
        data["grant_number"] = fund_parts[index + 1]
        
        if "funded by Ministry of Science and ICT" in fund_string:
            agency_index = fund_string.find("Ministry of Science and ICT") - len("funded by ")
            data["funding_agency"] = fund_string[agency_index: fund_parts.index("NRF")]
            
    if "project title" in fund_string.lower():
        index = fund_string.lower().find("project title")
        start = fund_string.find(": ") + 2
        end = fund_string.find(".", start)
        data["project_title"] = fund_string[start:end]
    
    return json.dumps(data, indent=4)

funding_signal = "This research was supported by National R&D Program through the National Research Foundation of Korea (NRF) funded by Ministry of Science and ICT (Grant No. NRF-2021M3F3A2A01037844)."
print(summarize_funding(funding_signal))