int main(void) {
    int n;
    n = 0;
again:
    n = n + 1;
    if (n < 3) goto again;
    return n;
}
