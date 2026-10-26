void tick(void);
void stash(int **m);
int main(void) {
    int i = 0;
    while (i < 2) {
        int *m[1];
        if (i == 0) {
            stash(m);
        } else {
            tick();
            int *q = m[0];
            if (q != 0) { *q = 1; }
        }
        i = i + 1;
    }
    return 0;
}
