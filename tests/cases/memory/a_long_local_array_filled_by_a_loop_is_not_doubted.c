int main(void) {
    int x = 1;
    int *m[100];
    for (int i = 0; i < 100; i++) {
        m[i] = &x;
    }
    int *r = m[0];
    return *r;
}
