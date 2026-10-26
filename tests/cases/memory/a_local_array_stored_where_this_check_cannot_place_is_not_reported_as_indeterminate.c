int ***get_slot(void);
void tick(void);

int main(void) {
    int *m[1];
    int ***slot = get_slot();
    if (slot == 0) {
        return 1;
    }
    *slot = m;
    tick();
    int *q = m[0];
    return q == 0;
}
