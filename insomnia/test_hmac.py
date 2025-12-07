#!/usr/bin/env python3
"""
Утилита для вычисления HMAC для активации устройства
Использование: python test_hmac.py <challenge> [hmac_key]
"""

import sys
import hmac
import hashlib

def compute_hmac(challenge: str, hmac_key: str) -> str:
    """Вычисляет HMAC-SHA256 для challenge"""
    return hmac.new(
        hmac_key.encode('utf-8'),
        challenge.encode('utf-8'),
        hashlib.sha256
    ).hexdigest()

def main():
    if len(sys.argv) < 2:
        print("Использование: python test_hmac.py <challenge> [hmac_key]")
        print("\nПример:")
        print("  python test_hmac.py 'test-challenge-123' 'FCDEfd3_fde3d3fcelcvmfdjk646cfe32'")
        sys.exit(1)
    
    challenge = sys.argv[1]
    hmac_key = sys.argv[2] if len(sys.argv) > 2 else "FCDEfd3_fde3d3fcelcvmfdjk646cfe32"
    
    response = compute_hmac(challenge, hmac_key)
    
    print(f"Challenge: {challenge}")
    print(f"HMAC Key: {hmac_key}")
    print(f"Response: {response}")
    print(f"\nJSON для Insomnia:")
    print(f'{{"serial_number": "SN123456789", "challenge": "{challenge}", "response": "{response}"}}')

if __name__ == "__main__":
    main()

