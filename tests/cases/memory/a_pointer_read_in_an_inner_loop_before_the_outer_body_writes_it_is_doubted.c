int main(void) {
    int *m[2];
    int x = 1;
    int *r = 0;
    for (int i = 0; i < 2; i++) {
        for (int j = 0; j < 2; j++) {
            r = m[0];
        }
        m[0] = &x;
    }
    return r == 0;
}
