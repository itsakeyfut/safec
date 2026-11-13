int main(void) {
    int x = 1;
    int *m[65];
    m[0] = &x;
    int *q = m[1];
    return q == 0;
}
