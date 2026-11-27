struct P {
    int x, y;
};

int n;
struct P no_member = {.z = 1};
int past[3] = {[3] = 1};
int negative[3] = {[-1] = 1};
int not_constant[3] = {[n] = 1};
struct P index_of_a_struct = {[0] = 1};
int member_of_an_array[2] = {.x = 1};
int scalar = {.x = 1};
