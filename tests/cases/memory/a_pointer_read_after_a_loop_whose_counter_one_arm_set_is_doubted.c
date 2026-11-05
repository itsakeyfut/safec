int c(void);

int main(void) {
    int x = 1;
    int *m[1];
    int i = 3;
    if (c()) {
        i = 0;
    }
    while (i < 2) {
        m[0] = &x;
        i = i + 1;
    }
    int *q = m[0];
    return q == 0;
}
