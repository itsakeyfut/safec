int in_a_block(void) {
    static int *p = (int[]){1};
    return *p;
}

int of_void(void) {
    (void){0};
    return 0;
}

int too_many(void) {
    return (int[1]){1, 2}[0];
}
