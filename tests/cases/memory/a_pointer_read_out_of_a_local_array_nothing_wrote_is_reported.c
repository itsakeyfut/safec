int main(void) {
    int *m[1];
    int *q = m[0];
    if (q == 0) {
        return 1;
    }
    *q = 1;
    return 0;
}
