void fill(int **s);
void tick(void);
int main(int c) {
    int *m[2];
    if (c) {
        fill(m + 1);
    }
    tick();
    int *r = m[0];
    return r != 0;
}
