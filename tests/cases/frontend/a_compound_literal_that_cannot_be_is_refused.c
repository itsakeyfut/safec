struct U;

int f(void);

int *non_constant = (int[]){f()};

int in_a_block(void) {
    static int *p = (int[]){1};
    return *p;
}

int of_void(void) {
    (void){0};
    return 0;
}

int of_a_function(void) {
    (int(void)){0};
    return 0;
}

int too_many(void) {
    return (int[1]){1, 2}[0];
}

int incomplete(void) {
    (struct U){0};
    (struct U[1]){0};
    return 0;
}

int an_array_from_one(void) {
    int a[2] = (int[2]){1, 2};
    return a[0];
}

int written_though_const(void) {
    (const int){0} = 1;
    return 0;
}
