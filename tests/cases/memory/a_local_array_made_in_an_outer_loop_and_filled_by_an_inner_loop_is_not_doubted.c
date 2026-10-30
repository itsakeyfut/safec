int main(void) {
    int x = 1;
    int r = 0;
    for (int i = 0; i < 2; i++) {
        int *m[2];
        for (int j = 0; j < 2; j++) {
            m[0] = &x;
        }
        r += m[0] == 0;
    }
    return r;
}
