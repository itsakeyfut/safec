int main(void) {
    int x = 1;
    int *m[2];
    m[1] = &x;
    int *q = m[1];
    return *q;
}
