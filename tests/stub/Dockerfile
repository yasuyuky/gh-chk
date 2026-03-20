FROM python:3.12-alpine

WORKDIR /app
COPY tests/stub/server.py /app/server.py
COPY tests/data /app/data

ENV GH_CHK_STUB_PORT=8080

EXPOSE 8080

CMD ["sh", "-c", "python3 /app/server.py --port ${GH_CHK_STUB_PORT}"]
