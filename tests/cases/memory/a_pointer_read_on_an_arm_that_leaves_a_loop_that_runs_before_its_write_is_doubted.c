int c(void);

int main(void) {
    int x = 1;
    int *m[1];
    for (int i = 0; i < 2; i++) {
        if (c()) {
            int *q = m[0];
            return q == 0;
        }
        m[0] = &x;
    }
    return 0;
}
