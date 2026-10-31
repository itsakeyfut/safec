int main(int argc, char **argv) {
    int x = 1;
    int *q;
    int **pq = &q;
    int *r = 0;
    for (int i = 0; i < 2; i++) {
        r = q;
        if (argc > 1) {
            q = &x;
        }
    }
    return r == 0;
}
