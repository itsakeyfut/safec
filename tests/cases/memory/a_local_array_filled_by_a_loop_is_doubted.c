int main(void) {
    int x = 1;
    int *m[4];
    for (int i = 0; i < 4; i++) {
        m[i] = &x;
    }
    int *r = m[0];
    return *r;
}
