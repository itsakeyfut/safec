int c(void);
int main(void) {
    int *m[100];
    int x = 1;
    for (int i = 0; i < 2; i++) {
        if (c()) {
            m[0] = &x;
        }
    }
    int *r = m[0];
    return r == 0;
}
