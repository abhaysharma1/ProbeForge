from fastapi import FastAPI

app = FastAPI()


@app.get("/fixture")
def fixture() -> dict[str, bool]:
    return {"inert": True}
