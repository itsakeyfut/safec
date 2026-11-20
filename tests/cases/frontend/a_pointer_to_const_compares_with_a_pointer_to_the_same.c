int main(void) {
    int x;
    const int *c;
    int *n;
    c = &x;
    n = &x;
    x = c < n;
    x = c - n;
    x = c == n;
    c = 1 ? c : n;
    return x;
}
