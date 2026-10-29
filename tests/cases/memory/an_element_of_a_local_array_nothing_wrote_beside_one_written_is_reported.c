int main(void) {
    int x = 1;
    int *m[2];
    m[0] = &x;
    int *q = m[1];
    return q == 0;
}
