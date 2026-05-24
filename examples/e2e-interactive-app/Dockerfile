FROM python:3.12-slim
# Unbuffered stdout so log lines reach `docker logs` (and thus
# Ruscker's live-logs follow stream) immediately instead of
# sitting in Python's block buffer.
ENV PYTHONUNBUFFERED=1
RUN pip install --no-cache-dir aiohttp
COPY app.py /app.py
EXPOSE 8080
CMD ["python", "/app.py"]
