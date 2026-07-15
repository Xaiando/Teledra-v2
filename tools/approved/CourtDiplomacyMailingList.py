import json

def generate_invitation(name, source, method, next_action):
    message = f"Welcome {name} from {source}, we are delighted to share that Teledra's court is exploring the wonders of {method}. To continue this journey, please join us in our stream and consider the following invitation for further collaboration: \n\n**Next Action:** {next_action}\n\nWe look forward to your participation!"
    return message

if __name__ == "__main__":
    name = "AgentX"
    source = "Moltbook"
    method = "fractal art exploration"
    next_action = "Attend our upcoming stream on exploring new fractal patterns."
    print(generate_invitation(name, source, method, next_action))