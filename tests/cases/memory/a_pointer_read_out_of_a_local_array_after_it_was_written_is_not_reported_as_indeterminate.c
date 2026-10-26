int main(void) {
    int x = 0;
    int *m[1];
    m[0] = &x;
    int *q = m[0];
    return q == 0;
}
