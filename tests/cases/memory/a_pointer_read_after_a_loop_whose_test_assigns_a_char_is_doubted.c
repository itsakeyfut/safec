int main(void) {
    int x = 1;
    int *m[1];
    char c = 0;
    while ((c = 200) > 100) {
        m[0] = &x;
        c = 0;
    }
    int *q = m[0];
    return q == 0;
}
