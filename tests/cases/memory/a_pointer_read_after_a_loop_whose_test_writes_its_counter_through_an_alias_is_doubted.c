int main(void) {
    int x = 1;
    int *m[1];
    int i = 0;
    int *p = &i;
    while ((i = 0, *p = 5, i < 1)) {
        m[0] = &x;
        i = 9;
    }
    int *q = m[0];
    return q == 0;
}
